#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK_DIR="${ROOT_DIR}/benchmarks/.work"
RESULTS_DIR="${ROOT_DIR}/benchmarks/results"
FOLDER_PORT="${FOLDER_PORT:-3100}"
JSON_SERVER_PORT="${JSON_SERVER_PORT:-3101}"
DURATION="${DURATION:-10}"
CONNECTIONS="${CONNECTIONS:-50}"
AMOUNT="${AMOUNT:-10000}"
RUNS="${RUNS:-3}"
WARMUP_DURATION="${WARMUP_DURATION:-3}"
WARMUP_CONNECTIONS="${WARMUP_CONNECTIONS:-1}"

mkdir -p "${WORK_DIR}" "${RESULTS_DIR}"

cleanup() {
  if [[ -n "${FOLDER_PID:-}" ]]; then kill "${FOLDER_PID}" 2>/dev/null || true; fi
  if [[ -n "${JSON_SERVER_PID:-}" ]]; then kill "${JSON_SERVER_PID}" 2>/dev/null || true; fi
}
trap cleanup EXIT

python3 - <<PY
import json
from pathlib import Path

amount = int(${AMOUNT})
posts = [
    {
        "id": i,
        "title": f"Post {i}",
        "views": i % 1000,
        "author": f"Author {i % 50}"
    }
    for i in range(1, amount + 1)
]

work = Path("${WORK_DIR}")
folder_data = work / "folder_data"
folder_data.mkdir(parents=True, exist_ok=True)
(folder_data / "posts.json").write_text(json.dumps(posts), encoding="utf-8")
(work / "db.json").write_text(json.dumps({"posts": posts}), encoding="utf-8")
PY

pushd "${ROOT_DIR}" >/dev/null
cargo build --release >/dev/null
popd >/dev/null

"${ROOT_DIR}/target/release/folder-server" --folder "${WORK_DIR}/folder_data" --bind "127.0.0.1:${FOLDER_PORT}" >"${WORK_DIR}/folder-server.log" 2>&1 &
FOLDER_PID=$!

npx --yes json-server@0.17.4 --host 127.0.0.1 --port "${JSON_SERVER_PORT}" --quiet "${WORK_DIR}/db.json" >"${WORK_DIR}/json-server.log" 2>&1 &
JSON_SERVER_PID=$!

for _ in {1..50}; do
  if curl -fsS "http://127.0.0.1:${FOLDER_PORT}/posts/1" >/dev/null 2>&1 && \
     curl -fsS "http://127.0.0.1:${JSON_SERVER_PORT}/posts/1" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done

STAMP="$(date +%Y%m%d-%H%M%S)"
SUMMARY_JSON="${RESULTS_DIR}/summary-${STAMP}.json"

run_autocannon() {
  local mode="$1"
  local run="$2"
  local target="$3"
  local url="$4"
  local out="${RESULTS_DIR}/${target}-${mode}-run${run}-${STAMP}.json"
  local -a cmd=(npx --yes autocannon@7.15.0 -n -c "${CONNECTIONS}" -d "${DURATION}" -j)

  if [[ "${mode}" == "with-warmup" ]]; then
    cmd+=(-W --warmup "[" -c "${WARMUP_CONNECTIONS}" -d "${WARMUP_DURATION}" "]")
  fi

  cmd+=("${url}")
  "${cmd[@]}" >"${out}"
}

for mode in with-warmup without-warmup; do
  for run in $(seq 1 "${RUNS}"); do
    run_autocannon "${mode}" "${run}" "folder-item" "http://127.0.0.1:${FOLDER_PORT}/posts/5000"
    run_autocannon "${mode}" "${run}" "json-server-item" "http://127.0.0.1:${JSON_SERVER_PORT}/posts/5000"
    run_autocannon "${mode}" "${run}" "folder-query" "http://127.0.0.1:${FOLDER_PORT}/posts?author:eq=Author%2010"
    run_autocannon "${mode}" "${run}" "json-server-query" "http://127.0.0.1:${JSON_SERVER_PORT}/posts?author=Author%2010"
  done
done

python3 - <<PY
import json
import statistics
from pathlib import Path

results_dir = Path("${RESULTS_DIR}")
stamp = "${STAMP}"

modes = ("with_warmup", "without_warmup")
targets = {
    "folder_item": "folder-item",
    "json_item": "json-server-item",
    "folder_query": "folder-query",
    "json_query": "json-server-query",
}


def summarize(metric_runs):
    return {
        "mean": statistics.fmean(metric_runs),
        "median": statistics.median(metric_runs),
        "min": min(metric_runs),
        "max": max(metric_runs),
    }


def load_autocannon_result(path: Path):
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    for line in reversed(lines):
        try:
            parsed = json.loads(line)
            if isinstance(parsed, dict) and "requests" in parsed and "latency" in parsed:
                return parsed
        except json.JSONDecodeError:
            continue
    raise RuntimeError(f"Failed to parse autocannon JSON result from {path}")


summary = {
    "config": {
        "runs": int("${RUNS}"),
        "duration": int("${DURATION}"),
        "connections": int("${CONNECTIONS}"),
        "warmup_duration": int("${WARMUP_DURATION}"),
        "warmup_connections": int("${WARMUP_CONNECTIONS}"),
        "amount": int("${AMOUNT}"),
    },
    "modes": {},
}

for mode in modes:
    mode_key = mode.replace("_", "-")
    mode_summary = {}
    for summary_key, target_key in targets.items():
        run_results = []
        for run in range(1, int("${RUNS}") + 1):
            path = results_dir / f"{target_key}-{mode_key}-run{run}-{stamp}.json"
            data = load_autocannon_result(path)
            run_results.append(
                {
                    "run": run,
                    "requests_per_sec": data["requests"]["average"],
                    "latency_ms": data["latency"]["average"],
                    "throughput_bytes_per_sec": data["throughput"]["average"],
                    "non_2xx": data.get("non2xx", 0),
                    "errors": data.get("errors", 0),
                    "timeouts": data.get("timeouts", 0),
                }
            )

        mode_summary[summary_key] = {
            "aggregate": {
                "requests_per_sec": summarize([r["requests_per_sec"] for r in run_results]),
                "latency_ms": summarize([r["latency_ms"] for r in run_results]),
                "throughput_bytes_per_sec": summarize([r["throughput_bytes_per_sec"] for r in run_results]),
                "non_2xx": int(sum(r["non_2xx"] for r in run_results)),
                "errors": int(sum(r["errors"] for r in run_results)),
                "timeouts": int(sum(r["timeouts"] for r in run_results)),
            },
            "runs": run_results,
        }

    summary["modes"][mode] = mode_summary

summary_path = Path("${SUMMARY_JSON}")
summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
print(summary_path)
PY
