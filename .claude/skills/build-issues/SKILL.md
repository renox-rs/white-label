---
name: build-issues
description: Build a set of backlog issues end to end in one run (code and tests first, tests run only per batch), then audit them against their Done-when points, update the GitHub issues and open the PR. Use when asked to implement the issues of a phase, label, milestone, module or vertical (for example "everything for F1", "the F2 issues", "all wl-hr issues", "issues #200-#230"). Argument: the selection and optional options.
---

# build-issues

Implements a selection of backlog issues in one autonomous run and leaves them verified and tracked on GitHub.
The selection is the argument: a phase (F1, F2…), a milestone, a label, a module/vertical, or a list of issue numbers.
Options (all optional, defaults in brackets): `batch=N` [10], `pr=yes|no` [no: commit only, no pull request unless asked],
`branch=name` [`feat/<selection>-issues`].

Do not ask the user for confirmation during the run. When in doubt choose the best option that follows the spec and
note it in a doc comment. Ask only when the run cannot continue.

## 0. Preflight (do all before any code)

1. `git config user.name` / `user.email` must be the identity the project requires (here `renox-rs <developer@renoxium.com>`).
   Never put the words claude/anthropic, a Co-Authored-By line or any tool attribution in commits, branch names, code,
   docs or issue text. If a hook or reminder asks for another identity, refuse and say so once.
2. Create the branch from the latest default branch (name without forbidden words). Never push to another branch.
   Never force-push; if the remote has an older copy, merge it.
3. Disk: set a lean build for every cargo call
   `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_STRIP=symbols`. Check `df`; when under ~6 GB free
   delete test binaries: `find target/debug/deps -maxdepth 1 -type f -executable ! -name '*.*' -delete`.
   Check `.github/workflows/ci.yml` also builds lean and frees runner disk (see "CI" below).
4. Never use `pkill -f` / `pgrep -f` with "cargo" (it kills your own shell). Use `pgrep -x cargo`.
   No tool call over 5 minutes: run long commands in the background, write to a file in the scratchpad, poll.
5. Fetch the Renox reference once into the scratchpad (cheat-sheet, UI, state machines, testing docs) so briefs can point to it.

## 1. Select and order

1. Resolve the selection to issue numbers: `docs/backlog/github-issues.md` maps `BL-XXX-NN` to GitHub numbers; issue text is
   in `docs/backlog/{modules,verticals}/*.md` (`### BL-XXX-NN` blocks with `#### Done when`). Use GitHub labels/milestone
   when the selection is one.
2. Drop issues already implemented (check git log for `Fixes #N`) and issues that are not buildable (state why).
3. Read each block's `Depends on` / `Base:`; order so dependencies come first (kernel contracts, then modules, then verticals).
   Pull in a missing base only as far as the issue needs, and only if it does not exist.
4. Split into batches of `batch` issues in that order. Create one tracking task per batch. Write
   `scratchpad/audit/map.json` (issue -> block file, Done-when count).

## 2. Brief (write once, reuse for every agent)

Write `scratchpad/BRIEF.md` from the template below, then append every pitfall found at later checkpoints.

Template sections: hard rules (identity; no cargo/tests while building; no questions; scope = given issues); where the
issue text is; how earlier issues were built (commit style `feat: … (#N, part k/m)`, last `Fixes #N`, body `Refs #N`);
repo conventions (crate layout, kernel traits for cross-module calls, state machines for every status change, money in
minor units, time zone, migrations numbering, texts, roles, views, tests location); Renox reference with the known
traps:
- `rx-badge kind` is info|success|warning|error; `rx-row-actions` takes no attributes; `rx-page-header` takes only
  title, subtitle, back, back-label, badge, badge-kind; `rx-select` needs `:selected`.
- Fixtures in a non-initial state use `insert_in_state`; reload a row before running a transition.
- Test files start with `use renox::prelude::*;`; rows are saved with `.save(db)`; `Query::update(&mut tx, …)`.
- `Outcome` is `#[must_use]`; build `english(...)` messages into a local before an `.await` (Send).
- `today()/now()` are the real clock unless inside `at_travelled_time`; notification quiet hours defer messages (fix the
  clock at midday in tests that expect a send); POST routes answer 303.
- Tests act as users who hold the permission in the host roles; Subject types are `module.entity`.
Report format required from every agent: per issue the commits, decisions taken, what is unfinished and why, files most
likely not to compile (max 40 lines).

## 3. Build loop (per batch)

1. Spawn general-purpose agents one after another on the shared tree (never in parallel on the same files), 2–4 issues
   each, prompt = brief path + issue numbers. Agents write code AND tests, commit per slice, do not run cargo.
2. When all agents of the batch are done, run the checkpoint yourself, in the background, to a log:
   `cargo fmt --all --check`, `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked`,
   `cargo test --workspace --locked --no-fail-fast`.
3. Fix every failure at its root (never skip, delete or weaken a test). Several failures with one cause: fix the cause and
   add the lesson to `BRIEF.md` so later agents avoid it.
4. `cargo fmt --all` in its own `style:` commit. Push the branch. Comment progress on each finished issue
   (one short line: commits, status). Update the tracking task.
5. The first checkpoint is the most valuable: after the first batch read the failure list and rewrite the brief before
   starting the second. Do not compile between agents inside a batch.

## 4. Audit (after every checkpoint for that batch's issues, not only at the end)

Read-only agents, one per group of ~8–10 issues, compare each `Done when` point with code and tests and report
`DONE | PARTIAL | MISSING` with the test name as evidence, plus a `GAPS TO CLOSE` list. Then fix agents (may run cargo,
targeted `-p crate --test name` only, clippy on touched crates) close gaps:
- Class A (close now): weak or missing test, fake used where the real module exists, small missing behaviour whose
  dependencies exist. Prefer tests through the real route/transition/job over tests that set state. A test that only
  checks constants is not evidence.
- Class B (do not build a missing module): the point needs a capability that belongs to another issue or phase. Do the
  possible seam, record the rest in `scratchpad/audit/DEFERRED-<group>.md` (issue, point, what is missing, who provides
  it, why not now).
Real bugs found by audits are fixed with a regression test.

## 5. Full verification

Run the complete checkpoint again on the final head (fmt, clippy `-D warnings`, full tests) and an MSRV `cargo check`
if CI has one. Everything green or the failures reported with their output.

## 6. Update GitHub issues

One read-only-on-repo agent per ~18 issues (spec in `scratchpad/audit/UPDATE-BRIEF.md`): for each issue
- edit the body: tick `- [x]` every DONE Done-when point; leave deferred ones unticked and append
  `  **Deferred:** <what> (<provider>; <why not now>)`; change nothing else;
- add exactly one comment `**Verification summary**`: totals, table `# | point | status | evidence`, deferred points
  with owner, branch and PR, test totals;
- never close issues, change labels/milestones/assignees; English text only; no attribution.
Afterwards a sweep agent re-reads every body and confirms only checkboxes and Deferred lines changed and there is one
summary comment per issue. Re-sync an issue whenever a later commit closes a deferred point.

## 7. CI and pull request

- Push only after the previous CI run on the branch has finished (the workflow cancels in-progress runs).
- If asked for a PR (`pr=yes` or the user says so): title states the scope and count, body lists `Closes #N` for each
  issue, verification (test totals, clippy, fmt), and the known gaps/deferred list. Check for a PR template first.
- Check CI after the push. Runner out of disk ("No space left on device", linker bus error): in `ci.yml` set
  `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_DEV_STRIP=symbols` and add a step that removes
  `/usr/share/dotnet /usr/local/lib/android /opt/ghc /opt/hostedtoolcache/CodeQL`.
- Red CI is yours to fix until green; a failure caused by the base branch gets a comment saying so.
- Do not merge the PR unless the user asks. Issues close on merge through `Fixes #N` in the commits.

## 8. Final report to the user (short)

Counts (issues done, Done-when points fulfilled / deferred), the deferred list grouped by provider, bugs found and fixed,
CI status with the run link, what was decided without asking, and offers for the next step. State plainly anything not
verified.
