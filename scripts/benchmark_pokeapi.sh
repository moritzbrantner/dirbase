#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK_DIR="${ROOT_DIR}/benchmarks/.work"
DATA_DIR="${DATA_DIR:-${WORK_DIR}/pokeapi-json}"
RESULTS_DIR="${ROOT_DIR}/benchmarks/results"
SCENARIOS_FILE="${WORK_DIR}/pokeapi-scenarios.tsv"
FOLDER_PORT="${FOLDER_PORT:-3200}"
JSON_SERVER_PORT="${JSON_SERVER_PORT:-3201}"
JSON_SERVER_VERSION="${JSON_SERVER_VERSION:-1.0.0-beta.12}"
DURATION="${DURATION:-10}"
CONNECTIONS="${CONNECTIONS:-50}"
RUNS="${RUNS:-3}"
WARMUP_DURATION="${WARMUP_DURATION:-3}"
WARMUP_CONNECTIONS="${WARMUP_CONNECTIONS:-1}"
FORCE_REBUILD_DATA="${FORCE_REBUILD_DATA:-0}"

mkdir -p "${WORK_DIR}" "${RESULTS_DIR}"

cleanup() {
  if [[ -n "${FOLDER_PID:-}" ]]; then kill "${FOLDER_PID}" 2>/dev/null || true; fi
  if [[ -n "${JSON_SERVER_PID:-}" ]]; then kill "${JSON_SERVER_PID}" 2>/dev/null || true; fi
}
trap cleanup EXIT

GENERATOR_ARGS=(--output-dir "${DATA_DIR}")
if [[ "${FORCE_REBUILD_DATA}" == "1" ]]; then
  GENERATOR_ARGS+=(--force)
fi
python3 "${ROOT_DIR}/scripts/build_pokeapi_json.py" "${GENERATOR_ARGS[@]}"

if [[ ! -d "${DATA_DIR}/folder" || ! -f "${DATA_DIR}/db.json" || ! -f "${DATA_DIR}/metadata.json" ]]; then
  echo "PokeAPI benchmark data generation did not produce the expected files in ${DATA_DIR}" >&2
  exit 1
fi

cat >"${SCENARIOS_FILE}" <<'EOF'
pokemon-item	GET /pokemon/25	/pokemon/25	item
moves-item	GET /moves/500	/moves/500	item
encounters-item	GET /encounters/32000	/encounters/32000	item
pokemon-default	GET /pokemon?is_default=true	/pokemon?is_default=true	filter
species-generation	GET /pokemon_species?generation_id=1	/pokemon_species?generation_id=1	filter
moves-power	GET /moves?power:gte=100	/moves?power:gte=100	filter
species-flavor-language	GET /pokemon_species_flavor_text?language_id=9	/pokemon_species_flavor_text?language_id=9	filter
species-name-contains	GET /pokemon_species?identifier:contains=saur	/pokemon_species?identifier:contains=saur	filter
pokemon-sort	GET /pokemon?_sort=-base_experience,identifier	/pokemon?_sort=-base_experience,identifier	sort
encounters-sort-page	GET /encounters?_sort=-max_level,pokemon_id&_page=1&_per_page=100	/encounters?_sort=-max_level,pokemon_id&_page=1&_per_page=100	sort
pokemon-moves-page	GET /pokemon_moves?pokemon_id=25&_sort=level,order&_page=1&_per_page=200	/pokemon_moves?pokemon_id=25&_sort=level,order&_page=1&_per_page=200	composite
move-flavor-text-search	GET /move_flavor_text?flavor_text:contains=power&_page=1&_per_page=50	/move_flavor_text?flavor_text:contains=power&_page=1&_per_page=50	composite
EOF

pushd "${ROOT_DIR}" >/dev/null
cargo build --release >/dev/null
popd >/dev/null

"${ROOT_DIR}/target/release/folder-server" \
  --folder "${DATA_DIR}/folder" \
  --bind "127.0.0.1:${FOLDER_PORT}" \
  >"${WORK_DIR}/pokeapi-folder-server.log" 2>&1 &
FOLDER_PID=$!

npx --yes "json-server@${JSON_SERVER_VERSION}" \
  --host 127.0.0.1 \
  --port "${JSON_SERVER_PORT}" \
  "${DATA_DIR}/db.json" \
  >"${WORK_DIR}/pokeapi-json-server.log" 2>&1 &
JSON_SERVER_PID=$!

for _ in {1..100}; do
  if curl -fsS "http://127.0.0.1:${FOLDER_PORT}/pokemon/1" >/dev/null 2>&1 && \
     curl -fsS "http://127.0.0.1:${JSON_SERVER_PORT}/pokemon/1" >/dev/null 2>&1; then
    READY=1
    break
  fi
  sleep 0.2
done

if [[ "${READY:-0}" -ne 1 ]]; then
  echo "Servers did not become ready in time." >&2
  echo "--- folder-server log ---" >&2
  cat "${WORK_DIR}/pokeapi-folder-server.log" >&2 || true
  echo "--- json-server log ---" >&2
  cat "${WORK_DIR}/pokeapi-json-server.log" >&2 || true
  exit 1
fi

STAMP="$(date +%Y%m%d-%H%M%S)"
SUMMARY_JSON="${RESULTS_DIR}/pokeapi-summary-${STAMP}.json"
REPORT_MD="${RESULTS_DIR}/pokeapi-report-${STAMP}.md"

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
    while IFS=$'\t' read -r key _label path _category; do
      [[ -z "${key}" ]] && continue
      run_autocannon "${mode}" "${run}" "pokeapi-folder-${key}" \
        "http://127.0.0.1:${FOLDER_PORT}${path}"
      run_autocannon "${mode}" "${run}" "pokeapi-json-server-${key}" \
        "http://127.0.0.1:${JSON_SERVER_PORT}${path}"
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
    for key, label, path, category in reader:
        scenarios.append(
            {
                "key": key,
                "label": label,
                "path": path,
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


def collect(server_prefix: str, scenario_key: str, mode_key: str):
    run_results = []
    for run in range(1, runs + 1):
        path = results_dir / f"{server_prefix}-{scenario_key}-{mode_key}-run{run}-{stamp}.json"
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
    return {
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
    mode_results = {}
    for scenario in scenarios:
        mode_results[scenario["key"]] = {
            "label": scenario["label"],
            "path": scenario["path"],
            "category": scenario["category"],
            "folder": collect("pokeapi-folder", scenario["key"], mode_key),
            "json_server": collect("pokeapi-json-server", scenario["key"], mode_key),
        }
    summary["modes"][mode] = mode_results

summary_path = Path("${SUMMARY_JSON}")
summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
print(summary_path)
PY

python3 "${ROOT_DIR}/scripts/render_pokeapi_benchmark_report.py" \
  --summary "${SUMMARY_JSON}" \
  --output "${REPORT_MD}"

echo "Summary: ${SUMMARY_JSON}"
echo "Report: ${REPORT_MD}"
