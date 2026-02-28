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
FOLDER_ITEM_JSON="${RESULTS_DIR}/folder-item-${STAMP}.json"
JSON_ITEM_JSON="${RESULTS_DIR}/json-server-item-${STAMP}.json"
FOLDER_QUERY_JSON="${RESULTS_DIR}/folder-query-${STAMP}.json"
JSON_QUERY_JSON="${RESULTS_DIR}/json-server-query-${STAMP}.json"
SUMMARY_JSON="${RESULTS_DIR}/summary-${STAMP}.json"

npx --yes autocannon@7.15.0 -c "${CONNECTIONS}" -d "${DURATION}" -j "http://127.0.0.1:${FOLDER_PORT}/posts/5000" >"${FOLDER_ITEM_JSON}"
npx --yes autocannon@7.15.0 -c "${CONNECTIONS}" -d "${DURATION}" -j "http://127.0.0.1:${JSON_SERVER_PORT}/posts/5000" >"${JSON_ITEM_JSON}"

npx --yes autocannon@7.15.0 -c "${CONNECTIONS}" -d "${DURATION}" -j "http://127.0.0.1:${FOLDER_PORT}/posts?author:eq=Author%2010" >"${FOLDER_QUERY_JSON}"
npx --yes autocannon@7.15.0 -c "${CONNECTIONS}" -d "${DURATION}" -j "http://127.0.0.1:${JSON_SERVER_PORT}/posts?author=Author%2010" >"${JSON_QUERY_JSON}"

python3 - <<PY
import json
from pathlib import Path

results = {
    "folder_item": Path("${FOLDER_ITEM_JSON}"),
    "json_item": Path("${JSON_ITEM_JSON}"),
    "folder_query": Path("${FOLDER_QUERY_JSON}"),
    "json_query": Path("${JSON_QUERY_JSON}"),
}

summary = {}
for key, path in results.items():
    data = json.loads(path.read_text())
    summary[key] = {
        "requests_per_sec": data["requests"]["average"],
        "latency_ms": data["latency"]["average"],
        "throughput_bytes_per_sec": data["throughput"]["average"],
        "non_2xx": data.get("non2xx", 0),
        "errors": data.get("errors", 0),
        "timeouts": data.get("timeouts", 0),
    }

summary_path = Path("${SUMMARY_JSON}")
summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
print(summary_path)
PY
