#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  printf 'usage: %s <baseline-checkout> [candidate-checkout]\n' "$0" >&2
  exit 2
fi

baseline="$(cd "$1" && pwd)"
candidate="$(cd "${2:-.}" && pwd)"

cargo build --quiet --release --manifest-path "$baseline/Cargo.toml"
cargo build --quiet --release --manifest-path "$candidate/Cargo.toml"

primary_argv="$(python3 - "$baseline" <<'PY'
import json
import sys
from pathlib import Path
repo = Path(sys.argv[1])
print(json.dumps([
    "python3",
    str(repo / "scripts" / "architecture_benchmark.py"),
    "contract",
    "--binary",
    str(repo / "target" / "release" / "dirbase"),
]))
PY
)"

candidate_argv="$(python3 - "$candidate" <<'PY'
import json
import sys
from pathlib import Path
repo = Path(sys.argv[1])
print(json.dumps([
    "python3",
    str(repo / "scripts" / "architecture_benchmark.py"),
    "contract",
    "--binary",
    str(repo / "target" / "release" / "dirbase"),
]))
PY
)"

exec bash "$(dirname "$0")/moonlight.sh" run \
  --primary-argv "$primary_argv" \
  --candidate-argv "$candidate_argv" \
  --serial-targets \
  --compact
