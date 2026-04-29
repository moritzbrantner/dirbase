#!/usr/bin/env bash
set -euo pipefail

cargo test -- --test-threads=1

(
  cd ui
  bun run test
  bun run test:coverage
  bun run test:e2e
)

(
  cd js
  bun test
)
