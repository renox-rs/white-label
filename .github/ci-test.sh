#!/usr/bin/env bash
# Runs a part of the workspace's tests, deleting the test binaries after
# each run so they never all sit on disk together.
#
# Why: every file under a crate's `tests/` is its own binary that links the
# whole app. With several hundred of them (host alone has ~180), `cargo test
# --workspace` builds every binary before it runs one, and the runner's disk
# fills up ("No space left on device", linker "Bus error"). Run per package,
# and `host` in groups of test files, the largest set on disk at once is one
# group. CI runs the parts in parallel jobs (ci.yml), so the wait is the
# slowest part, not the sum.
#
# Usage:
#   .github/ci-test.sh modules [K N]  the wl-* packages (of shard K of N)
#   .github/ci-test.sh apps [K N]     the packages that are neither wl-* nor host
#   .github/ci-test.sh host K N       host's test files of shard K of N (K from 1);
#                                     shard 1 also runs host's lib, bins and doc tests
#   .github/ci-test.sh all            everything, one part after the other
# Shards are dealt round-robin from the sorted list, so every package or
# file is in exactly one shard of N.
# Every run goes even when one fails; the exit code is 1 if any failed.

set -uo pipefail

GROUP=${GROUP:-20}
failed=()

clean() {
  # Test binaries are the extension-less executables in target/debug/deps;
  # the libraries (.rlib, .rmeta, .so) stay, so nothing is rebuilt.
  find target/debug/deps -maxdepth 1 -type f -executable ! -name '*.*' -delete 2>/dev/null || true
}

run() {
  local label=$1
  shift
  echo "::group::$label"
  if ! cargo test --locked "$@"; then
    failed+=("$label")
  fi
  clean
  echo "::endgroup::"
  df -h / | tail -1
}

packages() {
  cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name' | sort
}

# The packages of shard $2 of $3 among those of stdin, each run alone.
run_packages() {
  local label=$1 shard=$2 shards=$3 list
  mapfile -t list
  for ((i = shard - 1; i < ${#list[@]}; i += shards)); do
    run "$label $shard/$shards: ${list[i]}" -p "${list[i]}"
  done
}

# (Fed by process substitution, not a pipe: a pipe would run the loop in a
# subshell and lose the failures it records.)
modules() {
  run_packages modules "${1:-1}" "${2:-1}" < <(packages | grep '^wl-')
}

apps() {
  run_packages apps "${1:-1}" "${2:-1}" < <(packages | grep -v '^wl-' | grep -vx host)
}

# host's test files of shard $1 of $2, dealt round-robin so neighbouring
# (similar) files spread over the shards, run in groups of $GROUP.
host() {
  local shard=$1 shards=$2
  if [ "$shard" -eq 1 ]; then
    run "host (lib, bins)" -p host --lib --bins
    run "host (doc)" -p host --doc
  fi
  local all mine=()
  mapfile -t all < <(ls host/tests/*.rs | xargs -n1 basename | sed 's/\.rs$//' | sort)
  for ((i = shard - 1; i < ${#all[@]}; i += shards)); do mine+=("${all[i]}"); done
  for ((i = 0; i < ${#mine[@]}; i += GROUP)); do
    local chunk=("${mine[@]:i:GROUP}") args=()
    for name in "${chunk[@]}"; do args+=(--test "$name"); done
    run "host $shard/$shards: ${chunk[0]} .. ${chunk[-1]}" -p host "${args[@]}"
  done
}

case "${1:-all}" in
  modules) modules "${2:-1}" "${3:-1}" ;;
  apps) apps "${2:-1}" "${3:-1}" ;;
  host) host "${2:?shard}" "${3:?shards}" ;;
  all) modules; apps; host 1 1 ;;
  *) echo "unknown part: $1" >&2; exit 2 ;;
esac

if [ ${#failed[@]} -gt 0 ]; then
  echo "Failed:"
  printf '  %s\n' "${failed[@]}"
  exit 1
fi
echo "All test runs passed."
