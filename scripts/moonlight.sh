#!/usr/bin/env bash
set -euo pipefail

root="$(git rev-parse --show-toplevel)"

if [[ -n "${MOONLIGHT_BIN:-}" ]]; then
  exec "$MOONLIGHT_BIN" "$@"
fi

if command -v moonlight >/dev/null 2>&1; then
  exec moonlight "$@"
fi

moonlight_root="${MOONLIGHT_ROOT:-$(dirname "$root")/moonlight}"
if [[ -f "$moonlight_root/Cargo.toml" ]]; then
  exec cargo run --quiet --manifest-path "$moonlight_root/Cargo.toml" \
    -p moonlight-cli --bin moonlight -- "$@"
fi

printf '%s\n' \
  'Moonlight is unavailable; set MOONLIGHT_BIN or MOONLIGHT_ROOT, or keep a sibling moonlight checkout' >&2
exit 127
