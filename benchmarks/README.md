# Benchmark: folder-server vs typicode/json-server

This benchmark compares request throughput and latency between:

- `folder-server` (this repository)
- `json-server` (`typicode/json-server` package)

## What is measured

The benchmark uses a deterministic synthetic workload across six resources:

- `organizations`
- `teams`
- `members`
- `projects`
- `tickets`
- `deployments`

The default profile contains 92,252 rows across those resources and exercises a broader mix of read paths:

1. Item lookups on `tickets` and `projects`
2. Equality and range filters on `teams`, `projects`, and `deployments`
3. Text search on `tickets.summary`
4. Sorted and paginated collection reads on `members`, `tickets`, and `deployments`
5. Composite filter + sort + pagination workloads

The script uses equivalent server-specific query syntax where `folder-server` and `json-server` differ.

## Run

From repo root:

```bash
scripts/benchmark_vs_json_server.sh
```

To force a fresh data rebuild:

```bash
FORCE_REBUILD_DATA=1 scripts/benchmark_vs_json_server.sh
```

Optional knobs:

```bash
DURATION=15 CONNECTIONS=100 RUNS=5 WARMUP_DURATION=3 WARMUP_CONNECTIONS=1 JSON_SERVER_VERSION=0.17.4 scripts/benchmark_vs_json_server.sh
```

The generated data cache lives under `benchmarks/.work/benchmark-data/`. You can also rebuild it directly:

```bash
python3 scripts/build_benchmark_data.py --force
```

## Output

Raw `autocannon` JSON and aggregated reports are written to:

- `benchmarks/results/<target>-with-warmup-run<run>-<timestamp>.json`
- `benchmarks/results/<target>-without-warmup-run<run>-<timestamp>.json`
- `benchmarks/results/benchmark-summary-<timestamp>.json`
- `benchmarks/results/benchmark-report-<timestamp>.md`

`<target>` is one of:

- `folder-<scenario>`
- `json-server-<scenario>`

## Notes

- `json-server` and `autocannon` are executed via `npx`.
- The script starts both servers locally and cleans up processes automatically.
- The benchmark runs each scenario repeatedly (`RUNS`, default `3`) in two modes: with warm-up and without warm-up.
- Aggregated metrics include mean/median/min/max; prefer median values for stable comparisons.
