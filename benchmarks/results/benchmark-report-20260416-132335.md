# Benchmark report: folder-server vs json-server

Generated: 2026-04-16 11:24:50 UTC

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
| `GET /tickets/24000` | item | 44,656.00 | 328.00 | 0.01x | 0.02 | 14.70 | 0.00x |
| `GET /projects/4000` | item | 48,272.00 | 1,139.00 | 0.02x | 0.01 | 3.91 | 0.00x |
| `Region + active filter on teams` | filter | 25,544.00 | 1,428.00 | 0.06x | 0.04 | 3.04 | 0.01x |
| `Risk and budget filter on projects` | filter | 1,070.00 | 169.00 | 0.16x | 4.16 | 28.79 | 0.14x |
| `Summary contains 'timeout'` | text | 69.00 | 7.00 | 0.10x | 63.44 | 424.43 | 0.15x |
| `Active members for a team, sorted page` | page | 1,946.00 | 84.00 | 0.04x | 1.96 | 57.71 | 0.03x |
| `Prod deployments sorted by duration` | sort | 213.00 | 36.00 | 0.17x | 22.69 | 125.09 | 0.18x |
| `Open high-priority tickets for one team` | composite | 184.00 | 35.00 | 0.19x | 26.35 | 133.20 | 0.20x |
| `Failed prod deployments with rollback` | filter | 323.00 | 271.00 | 0.84x | 14.94 | 17.77 | 0.84x |

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
| `GET /tickets/24000` | item | 48,496.00 | 399.00 | 0.01x | 0.01 | 11.93 | 0.00x |
| `GET /projects/4000` | item | 54,832.00 | 1,141.00 | 0.02x | 0.01 | 3.92 | 0.00x |
| `Region + active filter on teams` | filter | 27,080.00 | 1,424.00 | 0.05x | 0.03 | 3.07 | 0.01x |
| `Risk and budget filter on projects` | filter | 1,095.00 | 154.00 | 0.14x | 4.06 | 31.47 | 0.13x |
| `Summary contains 'timeout'` | text | 74.00 | 9.00 | 0.12x | 65.80 | 418.67 | 0.16x |
| `Active members for a team, sorted page` | page | 4,874.00 | 130.00 | 0.03x | 0.40 | 37.21 | 0.01x |
| `Prod deployments sorted by duration` | sort | 263.00 | 56.00 | 0.21x | 18.33 | 84.67 | 0.22x |
| `Open high-priority tickets for one team` | composite | 186.00 | 32.00 | 0.17x | 25.69 | 145.07 | 0.18x |
| `Failed prod deployments with rollback` | filter | 367.00 | 280.00 | 0.76x | 12.96 | 17.17 | 0.75x |

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

