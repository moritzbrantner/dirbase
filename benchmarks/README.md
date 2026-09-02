# Benchmark: dirbase vs typicode/json-server

This benchmark compares request throughput, latency, and behavioral parity between:

- `dirbase` (this repository)
- `json-server` (`typicode/json-server` package)

The benchmark suite is the implementation-comparison oracle. `runtime-profiler` and Moonlight sit around that suite as evidence tooling: runtime-profiler captures reproducible execution evidence, while Moonlight compares a baseline checkout with a candidate checkout. Neither tool replaces the response-correctness checks or the direct dirbase-vs-json-server throughput comparison.

## What is measured

The benchmark uses a deterministic synthetic workload across six resources:

- `organizations`
- `teams`
- `members`
- `projects`
- `tickets`
- `deployments`

The default profile contains 92,252 rows across those resources and exercises a broader mix of read and write paths:

1. Item lookups on `tickets` and `projects`
2. Equality and range filters on `teams`, `projects`, and `deployments`
3. Text search on `tickets.summary`
4. Sorted and paginated collection reads on `members`, `tickets`, and `deployments`
5. Composite filter + sort + pagination workloads
6. Query correctness checks that compare equivalent `dirbase` and `json-server` read responses
7. Concurrent `POST /members`, `PUT /members/{id}`, `PATCH /members/{id}`, and `DELETE /write_delete_items/{id}` workloads
8. Persisted JSON correctness checks after the write workloads

The script uses equivalent server-specific query syntax where `dirbase` and `json-server` differ.
For paginated reads, `dirbase` responses are normalized to their `data` array before comparison with the `json-server` array response. Other item, object, and array responses are compared exactly.

## Run

From repo root:

```bash
bash scripts/benchmark_vs_json_server.sh
```

For a faster parity/evidence loop that keeps all read scenarios and the response oracle but skips writes:

```bash
bash scripts/benchmark-parity-smoke.sh
```

To force a fresh data rebuild:

```bash
FORCE_REBUILD_DATA=1 bash scripts/benchmark_vs_json_server.sh
```

Optional knobs:

```bash
DURATION=15 CONNECTIONS=100 RUNS=5 WARMUP_DURATION=3 WARMUP_CONNECTIONS=1 JSON_SERVER_VERSION=0.17.4 bash scripts/benchmark_vs_json_server.sh
```

Write workload knobs:

```bash
WRITE_REQUESTS_PER_METHOD=1000 WRITE_CONNECTIONS=100 bash scripts/benchmark_vs_json_server.sh
```

To run only the read scenarios:

```bash
SKIP_WRITE_BENCHMARKS=1 bash scripts/benchmark_vs_json_server.sh
```

The generated data cache lives under `benchmarks/.work/benchmark-data/`. You can also rebuild it directly:

```bash
python3 scripts/build_benchmark_data.py --force
```

## Runtime evidence

The repository declares `profiles/runtime-profiler/json-server-parity.json`. Capture it into a fresh immutable output directory:

```bash
bash scripts/runtime-profile.sh .artifacts/runtime-profiler/parity-001
```

The scenario runs the parity smoke workload and therefore records the reproducibility, wall-time, success, source revision, environment fingerprint, and process evidence provided by runtime-profiler.

Important boundary: the profiler's process RSS measurement belongs to the benchmark harness process. It must not be presented as the resident-memory peak of either server. Throughput, latency, and behavioral equality between the two servers remain measurements produced by the benchmark suite itself. A future server-process collector can add direct per-server CPU/RSS evidence without changing this contract.

## Moonlight baseline/candidate comparison

Use Moonlight when evaluating whether a dirbase change improved or regressed the deterministic parity workload:

```bash
bash scripts/moonlight-compare.sh /path/to/baseline-checkout /path/to/candidate-checkout
```

The baseline and candidate each execute `benchmark-parity-smoke.sh`. That means Moonlight evaluates revisions of dirbase while each revision still compares itself against the same pinned JSON Server reference implementation.

The GitHub benchmark workflow exposes the same model. Pull requests get a short read-only parity run; the weekly/manual run uses the full benchmark; manual dispatch can additionally provide `baseline_ref` to run Moonlight. Weekly/manual runs also capture runtime-profiler evidence.

## Output

Raw `autocannon` JSON and aggregated reports are written to:

- `benchmarks/results/<target>-with-warmup-run<run>-<timestamp>.json`
- `benchmarks/results/<target>-without-warmup-run<run>-<timestamp>.json`
- `benchmarks/results/write-folder-<timestamp>.json`
- `benchmarks/results/write-json-server-<timestamp>.json`
- `benchmarks/results/query-correctness-<timestamp>.json`
- `benchmarks/results/benchmark-summary-<timestamp>.json`
- `benchmarks/results/benchmark-report-<timestamp>.md`

Runtime-profiler evidence is written below the fresh directory supplied to `runtime-profile.sh`. Moonlight output is captured by the workflow as an artifact and job summary.

`<target>` is one of:

- `folder-<scenario>`
- `json-server-<scenario>`
- `write-folder`
- `write-json-server`

## Notes

- `json-server` is pinned by `JSON_SERVER_VERSION` (default `0.17.4`) so parity evidence does not silently move underneath a candidate.
- `json-server` and `autocannon` are executed via `bunx --bun`.
- The script starts both servers locally and cleans up processes automatically.
- The full benchmark runs each scenario repeatedly (`RUNS`, default `3`) in two modes: with warm-up and without warm-up.
- The write phase uses an isolated copy under `benchmarks/.work/write-benchmark-data/` so read data is not mutated.
- Aggregated metrics include mean/median/min/max; prefer median values for stable comparisons.
- Correctness failures are benchmark failures, not merely performance annotations. A faster response with different normalized JSON is not parity.
