#!/usr/bin/env bash
# Runs the workspace's tests one package at a time and deletes the test
# binaries after each run, so they never all sit on disk together.
#
# Why: every file under a crate's `tests/` is its own binary that links the
# whole app. With several hundred of them (host alone has ~180), `cargo test
# --workspace` builds every binary before it runs one, and the runner's disk
# fills up ("No space left on device", linker "Bus error"). Run per package,
# and `host` in groups of test files, the largest set on disk at once is one
# group.
#
# Usage: .github/ci-test.sh [group size for host, default 20]
# Every package runs even when one fails; the exit code is 1 if any failed.

set -uo pipefail

GROUP=${1:-20}
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

packages=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name' | sort)

for package in $packages; do
  [ "$package" = host ] && continue
  run "$package" -p "$package"
done

# host: the library and binary unit tests, its doc tests, then its test
# files in groups.
run "host (lib, bins)" -p host --lib --bins
run "host (doc)" -p host --doc
mapfile -t files < <(ls host/tests/*.rs | xargs -n1 basename | sed 's/\.rs$//' | sort)
for ((i = 0; i < ${#files[@]}; i += GROUP)); do
  chunk=("${files[@]:i:GROUP}")
  args=()
  for name in "${chunk[@]}"; do args+=(--test "$name"); done
  run "host tests ${chunk[0]} .. ${chunk[-1]}" -p host "${args[@]}"
done

if [ ${#failed[@]} -gt 0 ]; then
  echo "Failed:"
  printf '  %s\n' "${failed[@]}"
  exit 1
fi
echo "All test runs passed."
