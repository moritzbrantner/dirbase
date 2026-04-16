# Benchmark report: folder-server vs json-server

Generated: 2026-04-16 11:27:01 UTC

## Dataset

- Dataset: `synthetic-workload`
- Resources: 6
- Total rows across all resources: 92,252
- Generated JSON folder: `/home/moenarch/moritzbrantner/folder-server/benchmarks/.work/benchmark-data/folder`
- Generated `db.json`: `/home/moenarch/moritzbrantner/folder-server/benchmarks/.work/benchmark-data/db.json`

### Resource sizes

| Resource | Rows |
|---|---:|
| `organizations` | 12 |
| `teams` | 240 |
| `members` | 12,000 |
| `projects` | 8,000 |
| `tickets` | 48,000 |
| `deployments` | 24,000 |

## Run configuration

- Repeated runs per scenario: 1
- Benchmark duration: 1s
- Connections: 5
- Warm-up: 1 connections for 1s
- json-server version: `0.17.4`
- Scenario count: 9

## Scenario set

- `GET /tickets/24000` (item): `/tickets/24000`
- `GET /projects/4000` (item): `/projects/4000`
- `Region + active filter on teams` (filter): `folder-server /teams?region:eq=us-west&active=true` vs `json-server /teams?region=us-west&active=true`
- `Risk and budget filter on projects` (filter): `folder-server /projects?risk_score:gte=70&budget_k:lte=400` vs `json-server /projects?risk_score_gte=70&budget_k_lte=400`
- `Summary contains 'timeout'` (text): `folder-server /tickets?summary:contains=timeout` vs `json-server /tickets?summary_like=timeout`
- `Active members for a team, sorted page` (page): `folder-server /members?team_id:eq=17&active=true&_sort=-tenure_years,id&_page=1&_per_page=100` vs `json-server /members?team_id=17&active=true&_sort=tenure_years,id&_order=desc,asc&_page=1&_limit=100`
- `Prod deployments sorted by duration` (sort): `folder-server /deployments?environment:eq=prod&_sort=-duration_ms,project_id&_page=1&_per_page=100` vs `json-server /deployments?environment=prod&_sort=duration_ms,project_id&_order=desc,asc&_page=1&_limit=100`
- `Open high-priority tickets for one team` (composite): `folder-server /tickets?team_id:eq=17&status:eq=open&priority:gte=4&_sort=-severity,due_at&_page=1&_per_page=100` vs `json-server /tickets?team_id=17&status=open&priority_gte=4&_sort=severity,due_at&_order=desc,asc&_page=1&_limit=100`
- `Failed prod deployments with rollback` (filter): `folder-server /deployments?environment:eq=prod&success=false&rollback=true` vs `json-server /deployments?environment=prod&success=false&rollback=true`

## With warm-up

| Scenario | Category | folder-server req/s | json-server req/s | json-server speedup | folder-server latency (ms) | json-server latency (ms) | folder-server slower |
|---|---|---:|---:|---:|---:|---:|---:|
| `GET /tickets/24000` | item | 58,832.00 | 408.00 | 0.01x | 0.01 | 11.65 | 0.00x |
| `GET /projects/4000` | item | 45,424.00 | 1,146.00 | 0.03x | 0.02 | 3.92 | 0.01x |
| `Region + active filter on teams` | filter | 30,296.00 | 1,423.00 | 0.05x | 0.01 | 3.05 | 0.00x |
| `Risk and budget filter on projects` | filter | 1,159.00 | 157.00 | 0.14x | 3.80 | 30.95 | 0.12x |
| `Summary contains 'timeout'` | text | 82.00 | 7.00 | 0.09x | 58.66 | 475.58 | 0.12x |
| `Active members for a team, sorted page` | page | 2,139.00 | 85.00 | 0.04x | 1.82 | 55.85 | 0.03x |
| `Prod deployments sorted by duration` | sort | 183.00 | 33.00 | 0.18x | 26.54 | 138.40 | 0.19x |
| `Open high-priority tickets for one team` | composite | 150.00 | 34.00 | 0.23x | 32.09 | 136.77 | 0.23x |
| `Failed prod deployments with rollback` | filter | 286.00 | 257.00 | 0.90x | 16.83 | 18.73 | 0.90x |

| Scenario | folder non-2xx | folder errors | folder timeouts | json-server non-2xx | json-server errors | json-server timeouts |
|---|---:|---:|---:|---:|---:|---:|
| `GET /tickets/24000` | 0 | 0 | 0 | 0 | 0 | 0 |
| `GET /projects/4000` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Region + active filter on teams` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Risk and budget filter on projects` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Summary contains 'timeout'` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Active members for a team, sorted page` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Prod deployments sorted by duration` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Open high-priority tickets for one team` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Failed prod deployments with rollback` | 0 | 0 | 0 | 0 | 0 | 0 |

## Without warm-up

| Scenario | Category | folder-server req/s | json-server req/s | json-server speedup | folder-server latency (ms) | json-server latency (ms) | folder-server slower |
|---|---|---:|---:|---:|---:|---:|---:|
| `GET /tickets/24000` | item | 47,376.00 | 396.00 | 0.01x | 0.02 | 12.06 | 0.00x |
| `GET /projects/4000` | item | 45,008.00 | 1,175.00 | 0.03x | 0.02 | 3.86 | 0.01x |
| `Region + active filter on teams` | filter | 25,656.00 | 1,427.00 | 0.06x | 0.03 | 3.06 | 0.01x |
| `Risk and budget filter on projects` | filter | 971.00 | 150.00 | 0.15x | 4.33 | 32.11 | 0.13x |
| `Summary contains 'timeout'` | text | 74.00 | 8.00 | 0.11x | 64.76 | 439.88 | 0.15x |
| `Active members for a team, sorted page` | page | 4,882.00 | 119.00 | 0.02x | 0.45 | 40.81 | 0.01x |
| `Prod deployments sorted by duration` | sort | 255.00 | 51.00 | 0.20x | 18.86 | 92.55 | 0.20x |
| `Open high-priority tickets for one team` | composite | 166.00 | 27.00 | 0.16x | 29.28 | 171.12 | 0.17x |
| `Failed prod deployments with rollback` | filter | 304.00 | 142.00 | 0.47x | 15.85 | 34.00 | 0.47x |

| Scenario | folder non-2xx | folder errors | folder timeouts | json-server non-2xx | json-server errors | json-server timeouts |
|---|---:|---:|---:|---:|---:|---:|
| `GET /tickets/24000` | 0 | 0 | 0 | 0 | 0 | 0 |
| `GET /projects/4000` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Region + active filter on teams` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Risk and budget filter on projects` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Summary contains 'timeout'` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Active members for a team, sorted page` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Prod deployments sorted by duration` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Open high-priority tickets for one team` | 0 | 0 | 0 | 0 | 0 | 0 |
| `Failed prod deployments with rollback` | 0 | 0 | 0 | 0 | 0 | 0 |

