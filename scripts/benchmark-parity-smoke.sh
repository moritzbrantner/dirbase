#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Keep this deliberately small enough for profiler/Moonlight loops while still
# exercising every read scenario and the response-parity oracle.
export DURATION="${DURATION:-1}"
export CONNECTIONS="${CONNECTIONS:-4}"
export RUNS="${RUNS:-1}"
export WARMUP_DURATION="${WARMUP_DURATION:-1}"
export WARMUP_CONNECTIONS="${WARMUP_CONNECTIONS:-1}"
export SKIP_WRITE_BENCHMARKS="${SKIP_WRITE_BENCHMARKS:-1}"

stamp="$(date +%Y%m%d-%H%M%S)-$$"
metrics_output="${SERVER_PROCESS_METRICS_OUTPUT:-${root}/benchmarks/results/server-process-metrics-smoke-${stamp}.json}"

exec python3 "${root}/scripts/profile_benchmark_servers.py" \
  --output "${metrics_output}" \
  -- bash "${root}/scripts/benchmark_vs_json_server.sh"
