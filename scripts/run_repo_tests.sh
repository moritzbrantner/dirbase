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

bash scripts/check_generated_ui_clean.sh

(
  cd js
  bun test
)
