# Benchmark: folder-server vs typicode/json-server

This benchmark compares request throughput and latency between:

- `folder-server` (this repository)
- `json-server` (`typicode/json-server` package)

## What is measured

The script runs two scenarios against both servers:

1. **Item lookup**: `GET /posts/5000`
2. **Filtered query**:
   - `folder-server`: `GET /posts?author:eq=Author%2010`
   - `json-server`: `GET /posts?author=Author%2010`

Data set is synthetic and contains `AMOUNT` rows (default: `10000`) with fields:
`id`, `title`, `views`, and `author`.

## Run

From repo root:

```bash
scripts/benchmark_vs_json_server.sh
```

Optional knobs:

```bash
DURATION=15 CONNECTIONS=100 AMOUNT=20000 scripts/benchmark_vs_json_server.sh
```

## Output

Raw `autocannon` JSON and aggregated summary are written to:

- `benchmarks/results/folder-item-<timestamp>.json`
- `benchmarks/results/json-server-item-<timestamp>.json`
- `benchmarks/results/folder-query-<timestamp>.json`
- `benchmarks/results/json-server-query-<timestamp>.json`
- `benchmarks/results/summary-<timestamp>.json`

## Notes

- `json-server` and `autocannon` are executed via `npx`.
- The script starts both servers locally and cleans up processes automatically.
- Use several runs and compare medians for stable conclusions.

## GitHub Actions workflow

You can also run benchmarks from GitHub Actions using the `Benchmarks` workflow (`.github/workflows/benchmarks.yml`).

1. Open **Actions** → **Benchmarks** → **Run workflow**.
2. Optionally set `duration`, `connections`, and `amount`.
3. After the run completes:
   - Read the generated markdown table in the workflow **Summary** tab.
   - Download `benchmark-results-<run_id>` from **Artifacts** for full JSON + markdown reports.

