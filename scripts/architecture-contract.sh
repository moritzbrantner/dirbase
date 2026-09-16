#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cargo build --quiet --release --manifest-path "$root/Cargo.toml"
exec python3 "$root/scripts/architecture_benchmark.py" contract \
  --binary "$root/target/release/dirbase"
