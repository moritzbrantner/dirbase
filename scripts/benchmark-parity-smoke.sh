#!/usr/bin/env bash
set -euo pipefail

# Keep this deliberately small enough for profiler/Moonlight loops while still
# exercising every read scenario and the response-parity oracle.
export DURATION="${DURATION:-1}"
export CONNECTIONS="${CONNECTIONS:-4}"
export RUNS="${RUNS:-1}"
export WARMUP_DURATION="${WARMUP_DURATION:-1}"
export WARMUP_CONNECTIONS="${WARMUP_CONNECTIONS:-1}"
export SKIP_WRITE_BENCHMARKS="${SKIP_WRITE_BENCHMARKS:-1}"

exec bash "$(dirname "$0")/benchmark_vs_json_server.sh"
