#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  printf 'usage: %s <baseline-checkout> [candidate-checkout]\n' "$0" >&2
  exit 2
fi

baseline="$(cd "$1" && pwd)"
candidate="$(cd "${2:-.}" && pwd)"
driver="$candidate/scripts/architecture_benchmark.py"

cargo build --quiet --release --manifest-path "$baseline/Cargo.toml"
cargo build --quiet --release --manifest-path "$candidate/Cargo.toml"

primary_argv="$(python3 - "$driver" "$baseline/target/release/dirbase" <<'PY'
import json
import sys
print(json.dumps([
    "python3",
    sys.argv[1],
    "contract",
    "--binary",
    sys.argv[2],
]))
PY
)"

candidate_argv="$(python3 - "$driver" "$candidate/target/release/dirbase" <<'PY'
import json
import sys
print(json.dumps([
    "python3",
    sys.argv[1],
    "contract",
    "--binary",
    sys.argv[2],
]))
PY
)"

exec bash "$(dirname "$0")/moonlight.sh" run \
  --primary-argv "$primary_argv" \
  --candidate-argv "$candidate_argv" \
  --serial-targets \
  --compact
