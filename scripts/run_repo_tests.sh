#!/usr/bin/env bash
set -euo pipefail

cargo test -- --test-threads=1
python3 scripts/render_test_matrix.py --check

(
  cd ui
  bun run typecheck
  bun run test
  bun run test:coverage
  bun run test:e2e
)

(
  cd js
  bun test
)
