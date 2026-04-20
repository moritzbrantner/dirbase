#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK_DIR="${ROOT_DIR}/benchmarks/.work"
DATA_DIR="${DATA_DIR:-${WORK_DIR}/benchmark-data}"
RESULTS_DIR="${ROOT_DIR}/benchmarks/results"
SCENARIOS_FILE="${WORK_DIR}/benchmark-scenarios.tsv"
FOLDER_PORT="${FOLDER_PORT:-}"
JSON_SERVER_PORT="${JSON_SERVER_PORT:-}"
JSON_SERVER_VERSION="${JSON_SERVER_VERSION:-0.17.4}"
DURATION="${DURATION:-10}"
CONNECTIONS="${CONNECTIONS:-50}"
RUNS="${RUNS:-3}"
WARMUP_DURATION="${WARMUP_DURATION:-3}"
WARMUP_CONNECTIONS="${WARMUP_CONNECTIONS:-1}"
FORCE_REBUILD_DATA="${FORCE_REBUILD_DATA:-0}"

mkdir -p "${WORK_DIR}" "${RESULTS_DIR}"

pick_free_port() {
  python3 - <<'PY'
import socket

with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

if [[ -z "${FOLDER_PORT}" ]]; then
  FOLDER_PORT="$(pick_free_port)"
fi

if [[ -z "${JSON_SERVER_PORT}" ]]; then
  JSON_SERVER_PORT="$(pick_free_port)"
fi

cleanup() {
  if [[ -n "${FOLDER_PID:-}" ]]; then kill "${FOLDER_PID}" 2>/dev/null || true; fi
  if [[ -n "${JSON_SERVER_PID:-}" ]]; then kill "${JSON_SERVER_PID}" 2>/dev/null || true; fi
}
trap cleanup EXIT

GENERATOR_ARGS=(--output-dir "${DATA_DIR}")
if [[ "${FORCE_REBUILD_DATA}" == "1" ]]; then
  GENERATOR_ARGS+=(--force)
fi
python3 "${ROOT_DIR}/scripts/build_benchmark_data.py" "${GENERATOR_ARGS[@]}"

if [[ ! -d "${DATA_DIR}/folder" || ! -f "${DATA_DIR}/db.json" || ! -f "${DATA_DIR}/metadata.json" ]]; then
  echo "Benchmark data generation did not produce the expected files in ${DATA_DIR}" >&2
  exit 1
fi

python3 - <<PY >"${SCENARIOS_FILE}"
import csv
import json
from pathlib import Path

metadata = json.loads(Path("${DATA_DIR}/metadata.json").read_text(encoding="utf-8"))
values = metadata["scenario_values"]

rows = [
    (
        "ticket-item",
        f"GET /tickets/{values['ticket_item_id']}",
        f"/tickets/{values['ticket_item_id']}",
        f"/tickets/{values['ticket_item_id']}",
        "item",
    ),
    (
        "project-item",
        f"GET /projects/{values['project_item_id']}",
        f"/projects/{values['project_item_id']}",
        f"/projects/{values['project_item_id']}",
        "item",
    ),
    (
        "teams-region-active",
        "Region + active filter on teams",
        f"/teams?region:eq={values['focus_region']}&active=true",
        f"/teams?region={values['focus_region']}&active=true",
        "filter",
    ),
    (
        "projects-risk-budget",
        "Risk and budget filter on projects",
        (
            f"/projects?risk_score:gte={values['project_risk_threshold']}"
            f"&budget_k:lte={values['project_budget_ceiling_k']}"
        ),
        (
            f"/projects?risk_score_gte={values['project_risk_threshold']}"
            f"&budget_k_lte={values['project_budget_ceiling_k']}"
        ),
        "filter",
    ),
    (
        "tickets-summary-search",
        f"Summary contains '{values['summary_term']}'",
        f"/tickets?summary:contains={values['summary_term']}",
        f"/tickets?summary_like={values['summary_term']}",
        "text",
    ),
    (
        "members-team-active-page",
        "Active members for a team, sorted page",
        (
            f"/members?team_id:eq={values['focus_team_id']}&active=true"
            f"&_sort=-tenure_years,id&_page=1&_per_page={values['page_size']}"
        ),
        (
            f"/members?team_id={values['focus_team_id']}&active=true"
            f"&_sort=tenure_years,id&_order=desc,asc&_page=1&_limit={values['page_size']}"
        ),
        "page",
    ),
    (
        "deployments-prod-sort-page",
        "Prod deployments sorted by duration",
        f"/deployments?environment:eq=prod&_sort=-duration_ms,project_id&_page=1&_per_page={values['page_size']}",
        f"/deployments?environment=prod&_sort=duration_ms,project_id&_order=desc,asc&_page=1&_limit={values['page_size']}",
        "sort",
    ),
    (
        "tickets-team-open-priority",
        "Open high-priority tickets for one team",
        (
            f"/tickets?team_id:eq={values['focus_team_id']}&status:eq=open"
            f"&priority:gte={values['priority_threshold']}"
            f"&_sort=-severity,due_at&_page=1&_per_page={values['page_size']}"
        ),
        (
            f"/tickets?team_id={values['focus_team_id']}&status=open"
            f"&priority_gte={values['priority_threshold']}"
            f"&_sort=severity,due_at&_order=desc,asc&_page=1&_limit={values['page_size']}"
        ),
        "composite",
    ),
    (
        "deployments-failure-rollbacks",
        "Failed prod deployments with rollback",
        "/deployments?environment:eq=prod&success=false&rollback=true",
        "/deployments?environment=prod&success=false&rollback=true",
        "filter",
    ),
]

with open("${SCENARIOS_FILE}", "w", encoding="utf-8", newline="") as handle:
    writer = csv.writer(handle, delimiter="\t")
    writer.writerows(rows)
PY

pushd "${ROOT_DIR}" >/dev/null
cargo build --release >/dev/null
popd >/dev/null

"${ROOT_DIR}/target/release/dirbase" \
  --folder "${DATA_DIR}/folder" \
  --bind "127.0.0.1:${FOLDER_PORT}" \
  >"${WORK_DIR}/dirbase.log" 2>&1 &
FOLDER_PID=$!

bunx --bun "json-server@${JSON_SERVER_VERSION}" \
  --host 127.0.0.1 \
  --port "${JSON_SERVER_PORT}" \
  --quiet \
  "${DATA_DIR}/db.json" \
  >"${WORK_DIR}/json-server.log" 2>&1 &
JSON_SERVER_PID=$!

for _ in {1..100}; do
  if curl -fsS "http://127.0.0.1:${FOLDER_PORT}/tickets/1" >/dev/null 2>&1 && \
     curl -fsS "http://127.0.0.1:${JSON_SERVER_PORT}/tickets/1" >/dev/null 2>&1; then
    READY=1
    break
  fi
  sleep 0.2
done

if [[ "${READY:-0}" -ne 1 ]]; then
  echo "Servers did not become ready in time." >&2
  echo "--- dirbase log ---" >&2
  cat "${WORK_DIR}/dirbase.log" >&2 || true
  echo "--- json-server log ---" >&2
  cat "${WORK_DIR}/json-server.log" >&2 || true
  exit 1
fi

STAMP="$(date +%Y%m%d-%H%M%S)"
SUMMARY_JSON="${RESULTS_DIR}/benchmark-summary-${STAMP}.json"
REPORT_MD="${RESULTS_DIR}/benchmark-report-${STAMP}.md"

run_autocannon() {
  local mode="$1"
  local run="$2"
  local target="$3"
  local url="$4"
  local out="${RESULTS_DIR}/${target}-${mode}-run${run}-${STAMP}.json"
  local -a cmd=(bunx --bun autocannon@7.15.0 -n -c "${CONNECTIONS}" -d "${DURATION}" -j)

  if [[ "${mode}" == "with-warmup" ]]; then
    cmd+=(-W --warmup "[" -c "${WARMUP_CONNECTIONS}" -d "${WARMUP_DURATION}" "]")
  fi

  cmd+=("${url}")
  "${cmd[@]}" >"${out}"
}

for mode in with-warmup without-warmup; do
  for run in $(seq 1 "${RUNS}"); do
    while IFS=$'\t' read -r key _label folder_path json_server_path _category; do
      [[ -z "${key}" ]] && continue
      run_autocannon "${mode}" "${run}" "folder-${key}" \
        "http://127.0.0.1:${FOLDER_PORT}${folder_path}"
      run_autocannon "${mode}" "${run}" "json-server-${key}" \
        "http://127.0.0.1:${JSON_SERVER_PORT}${json_server_path}"
    done <"${SCENARIOS_FILE}"
  done
done

python3 - <<PY
import csv
import json
import statistics
from pathlib import Path

results_dir = Path("${RESULTS_DIR}")
metadata = json.loads(Path("${DATA_DIR}/metadata.json").read_text(encoding="utf-8"))
stamp = "${STAMP}"
runs = int("${RUNS}")

scenarios = []
with open("${SCENARIOS_FILE}", "r", encoding="utf-8", newline="") as handle:
    reader = csv.reader(handle, delimiter="\t")
    for key, label, folder_path, json_server_path, category in reader:
        scenarios.append(
            {
                "key": key,
                "label": label,
                "path": folder_path if folder_path == json_server_path else f"folder: {folder_path} | json-server: {json_server_path}",
                "folder_path": folder_path,
                "json_server_path": json_server_path,
                "category": category,
            }
        )


def summarize(values):
    return {
        "mean": statistics.fmean(values),
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
    }


def load_autocannon_result(path: Path):
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    for line in reversed(lines):
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(parsed, dict) and "requests" in parsed and "latency" in parsed:
            return parsed
    raise RuntimeError(f"Failed to parse autocannon JSON result from {path}")


summary = {
    "config": {
        "runs": runs,
        "duration": int("${DURATION}"),
        "connections": int("${CONNECTIONS}"),
        "warmup_duration": int("${WARMUP_DURATION}"),
        "warmup_connections": int("${WARMUP_CONNECTIONS}"),
        "json_server_version": "${JSON_SERVER_VERSION}",
    },
    "dataset": metadata,
    "scenarios": scenarios,
    "modes": {},
}

for mode in ("with_warmup", "without_warmup"):
    mode_key = mode.replace("_", "-")
    mode_summary = {}
    for scenario in scenarios:
        folder_runs = []
        json_server_runs = []
        for run in range(1, runs + 1):
            folder_path = results_dir / f"folder-{scenario['key']}-{mode_key}-run{run}-{stamp}.json"
            json_server_path = results_dir / f"json-server-{scenario['key']}-{mode_key}-run{run}-{stamp}.json"
            folder_data = load_autocannon_result(folder_path)
            json_server_data = load_autocannon_result(json_server_path)

            folder_runs.append(
                {
                    "run": run,
                    "requests_per_sec": folder_data["requests"]["average"],
                    "latency_ms": folder_data["latency"]["average"],
                    "throughput_bytes_per_sec": folder_data["throughput"]["average"],
                    "non_2xx": folder_data.get("non2xx", 0),
                    "errors": folder_data.get("errors", 0),
                    "timeouts": folder_data.get("timeouts", 0),
                }
            )
            json_server_runs.append(
                {
                    "run": run,
                    "requests_per_sec": json_server_data["requests"]["average"],
                    "latency_ms": json_server_data["latency"]["average"],
                    "throughput_bytes_per_sec": json_server_data["throughput"]["average"],
                    "non_2xx": json_server_data.get("non2xx", 0),
                    "errors": json_server_data.get("errors", 0),
                    "timeouts": json_server_data.get("timeouts", 0),
                }
            )

        mode_summary[scenario["key"]] = {
            "label": scenario["label"],
            "path": scenario["path"],
            "category": scenario["category"],
            "folder": {
                "aggregate": {
                    "requests_per_sec": summarize([r["requests_per_sec"] for r in folder_runs]),
                    "latency_ms": summarize([r["latency_ms"] for r in folder_runs]),
                    "throughput_bytes_per_sec": summarize([r["throughput_bytes_per_sec"] for r in folder_runs]),
                    "non_2xx": int(sum(r["non_2xx"] for r in folder_runs)),
                    "errors": int(sum(r["errors"] for r in folder_runs)),
                    "timeouts": int(sum(r["timeouts"] for r in folder_runs)),
                },
                "runs": folder_runs,
            },
            "json_server": {
                "aggregate": {
                    "requests_per_sec": summarize([r["requests_per_sec"] for r in json_server_runs]),
                    "latency_ms": summarize([r["latency_ms"] for r in json_server_runs]),
                    "throughput_bytes_per_sec": summarize([r["throughput_bytes_per_sec"] for r in json_server_runs]),
                    "non_2xx": int(sum(r["non_2xx"] for r in json_server_runs)),
                    "errors": int(sum(r["errors"] for r in json_server_runs)),
                    "timeouts": int(sum(r["timeouts"] for r in json_server_runs)),
                },
                "runs": json_server_runs,
            },
        }
    summary["modes"][mode] = mode_summary

summary_path = Path("${SUMMARY_JSON}")
summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
print(summary_path)
PY

python3 "${ROOT_DIR}/scripts/render_benchmark_report.py" \
  --summary "${SUMMARY_JSON}" \
  --output "${REPORT_MD}"

echo "Summary: ${SUMMARY_JSON}"
echo "Report: ${REPORT_MD}"
