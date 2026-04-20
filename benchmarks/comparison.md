# Benchmark comparison snapshot

This file is a short snapshot of the latest local benchmark run. The generated report remains the source of truth for full configuration, scenarios, and raw aggregate tables.

- Latest report: [`benchmark-report-20260420-115936.md`](./results/benchmark-report-20260420-115936.md)
- Summary JSON: `benchmarks/results/benchmark-summary-20260420-115936.json`
- Generated: 2026-04-20 10:20:39 UTC
- Command: `FORCE_REBUILD_DATA=1 scripts/benchmark_vs_json_server.sh`
- Dataset: 92,252 rows across 6 resources
- Configuration: 3 runs per scenario, 10s duration, 50 connections, 3s warm-up at 1 connection, `json-server@0.17.4`
- Result quality: 0 non-2xx responses, 0 errors, and 0 timeouts across both benchmark modes

## Summary

Median throughput multiples, comparing `dirbase` request rate against `json-server`:

| Mode | Median `dirbase` throughput multiple | Range |
| --- | ---: | ---: |
| With warm-up | 15.08x | 5.37x-98.48x |
| Without warm-up | 16.92x | 4.85x-107.11x |

Median latency multiples, comparing `json-server` latency against `dirbase` latency:

| Mode | Median latency multiple | Range |
| --- | ---: | ---: |
| With warm-up | 13.10x | 5.41x-249.82x |
| Without warm-up | 13.62x | 4.87x-316.47x |

## Without Warm-up Medians

| Scenario | Category | dirbase req/s | json-server req/s | dirbase throughput multiple | dirbase latency (ms) | json-server latency (ms) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `GET /tickets/24000` | item | 55,795.20 | 520.90 | 107.11x | 0.30 | 94.94 |
| `GET /projects/4000` | item | 57,297.60 | 2,542.00 | 22.54x | 0.27 | 19.17 |
| `Region + active filter on teams` | filter | 34,898.40 | 3,827.30 | 9.12x | 1.16 | 12.56 |
| `Risk and budget filter on projects` | filter | 1,477.40 | 154.00 | 9.59x | 33.48 | 319.72 |
| `Summary contains 'timeout'` | text | 152.31 | 9.00 | 16.92x | 326.19 | 4,236.64 |
| `Active members for a team, sorted page` | page | 8,448.80 | 130.60 | 64.69x | 5.42 | 373.89 |
| `Prod deployments sorted by duration` | sort | 817.60 | 58.10 | 14.07x | 60.52 | 824.36 |
| `Open high-priority tickets for one team` | composite | 697.50 | 30.30 | 23.02x | 70.89 | 1,406.22 |
| `Failed prod deployments with rollback` | filter | 1,513.80 | 312.30 | 4.85x | 32.53 | 158.34 |

The benchmark uses the synthetic multi-resource workload described in [`benchmarks/README.md`](./README.md) and writes fresh markdown reports to `benchmarks/results/benchmark-report-<timestamp>.md`.

To generate a new comparison locally, run:

```bash
scripts/benchmark_vs_json_server.sh
```
