#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  printf 'usage: %s <fresh-output-directory> [scenario]\n' "$0" >&2
  exit 2
fi

root="$(git rev-parse --show-toplevel)"
output="$1"
scenario="${2:-$root/profiles/runtime-profiler/json-server-parity.json}"

if [[ -e "$output" ]]; then
  printf 'runtime-profiler output already exists: %s\n' "$output" >&2
  exit 2
fi

if [[ -n "${RUNTIME_PROFILER_BIN:-}" ]]; then
  exec "$RUNTIME_PROFILER_BIN" capture --scenario "$scenario" --output "$output"
fi

if command -v runtime-profiler >/dev/null 2>&1; then
  exec runtime-profiler capture --scenario "$scenario" --output "$output"
fi

profiler_root="${RUNTIME_PROFILER_ROOT:-$(dirname "$root")/runtime-profiler}"
if [[ -f "$profiler_root/Cargo.toml" ]]; then
  exec cargo run --quiet --manifest-path "$profiler_root/Cargo.toml" -- \
    capture --scenario "$scenario" --output "$output"
fi

printf '%s\n' \
  'runtime-profiler is unavailable; set RUNTIME_PROFILER_BIN or RUNTIME_PROFILER_ROOT, or keep a sibling runtime-profiler checkout' >&2
exit 127
