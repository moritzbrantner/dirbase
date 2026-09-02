#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  printf 'usage: %s <baseline-checkout> [candidate-checkout]\n' "$0" >&2
  exit 2
fi

baseline="$(cd "$1" && pwd)"
candidate="$(cd "${2:-.}" && pwd)"

printf -v primary 'cd %q && bash scripts/benchmark-parity-smoke.sh' "$baseline"
printf -v candidate_command 'cd %q && bash scripts/benchmark-parity-smoke.sh' "$candidate"

exec bash "$(dirname "$0")/moonlight.sh" run \
  --primary "$primary" \
  --candidate "$candidate_command"
