# Benchmark comparison: folder-server vs json-server

Run metadata:

- Script: `scripts/benchmark_vs_json_server.sh`
- Dataset size: 10,000 rows
- Load profile: 30 connections, 5s per scenario

## Results

| Scenario | folder-server req/s | json-server req/s | json-server speedup |
|---|---:|---:|---:|
| `GET /posts/5000` | 63.6 | 758.0 | 11.92x |
| filtered query | 95.4 | 355.6 | 3.73x |

| Scenario | folder-server latency avg (ms) | json-server latency avg (ms) | folder-server slower |
|---|---:|---:|---:|
| `GET /posts/5000` | 440.63 | 38.89 | 11.33x |
| filtered query | 305.10 | 83.37 | 3.66x |

## Interpretation

In this run, `json-server` outperformed `folder-server` in both tested read paths.
Because this benchmark does not include warm-up and uses one short trial, treat these numbers as directional.
Use repeated runs (and ideally median/p95 reporting) before drawing final conclusions.
