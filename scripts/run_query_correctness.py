#!/usr/bin/env python3
from __future__ import annotations

import argparse
import csv
import json
import socket
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qsl, urljoin, urlparse
from urllib.request import Request, urlopen


REQUEST_TIMEOUT_SECONDS = 30
PAGINATION_PARAMS = {"page", "_page", "per_page", "_per_page", "_limit"}


@dataclass(frozen=True)
class Scenario:
    key: str
    label: str
    folder_path: str
    json_server_path: str
    category: str


@dataclass(frozen=True)
class FetchResult:
    status: int | None
    payload: Any | None
    error: str | None = None


@dataclass(frozen=True)
class NormalizedPayload:
    value: Any
    count: int | None


def read_scenarios(path: Path) -> list[Scenario]:
    scenarios = []
    with path.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.reader(handle, delimiter="\t")
        for row in reader:
            if not row:
                continue
            if len(row) != 5:
                raise ValueError(f"Expected 5 columns in {path}, got {len(row)}: {row}")
            key, label, folder_path, json_server_path, category = row
            scenarios.append(
                Scenario(
                    key=key,
                    label=label,
                    folder_path=folder_path,
                    json_server_path=json_server_path,
                    category=category,
                )
            )
    return scenarios


def has_pagination(path: str) -> bool:
    query = urlparse(path).query
    return any(key in PAGINATION_PARAMS for key, _value in parse_qsl(query, keep_blank_values=True))


def fetch_json(base_url: str, path: str) -> FetchResult:
    request = Request(
        urljoin(f"{base_url.rstrip('/')}/", path.lstrip("/")),
        headers={"accept": "application/json"},
    )
    try:
        with urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            status = response.status
            body = response.read()
    except HTTPError as exc:
        return FetchResult(status=exc.code, payload=None, error=f"HTTP {exc.code}")
    except (TimeoutError, socket.timeout) as exc:
        return FetchResult(status=None, payload=None, error=f"timeout: {exc}")
    except URLError as exc:
        return FetchResult(status=None, payload=None, error=f"request failed: {exc.reason}")

    if status < 200 or status > 299:
        return FetchResult(status=status, payload=None, error=f"HTTP {status}")

    try:
        return FetchResult(status=status, payload=json.loads(body.decode("utf-8")))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        return FetchResult(status=status, payload=None, error=f"invalid JSON: {exc}")


def normalize_payload(payload: Any, *, source: str, expects_paginated: bool) -> NormalizedPayload:
    if expects_paginated and source == "folder":
        if not isinstance(payload, dict) or "data" not in payload:
            raise ValueError("dirbase paginated response is missing a data array")
        data = payload["data"]
        if not isinstance(data, list):
            raise ValueError("dirbase paginated response data is not an array")
        return NormalizedPayload(value=data, count=len(data))

    if isinstance(payload, list):
        return NormalizedPayload(value=payload, count=len(payload))

    return NormalizedPayload(value=payload, count=None)


def first_difference(left: Any, right: Any) -> str:
    if type(left) is not type(right):
        return f"type mismatch: {type(left).__name__} != {type(right).__name__}"

    if isinstance(left, list):
        if len(left) != len(right):
            return f"array length mismatch: {len(left)} != {len(right)}"
        for index, (left_item, right_item) in enumerate(zip(left, right, strict=True)):
            if left_item != right_item:
                return f"array item {index} differs"
        return "arrays differ"

    if isinstance(left, dict):
        left_keys = set(left)
        right_keys = set(right)
        if left_keys != right_keys:
            missing_left = sorted(right_keys - left_keys)
            missing_right = sorted(left_keys - right_keys)
            parts = []
            if missing_left:
                parts.append(f"missing in dirbase: {missing_left[:5]}")
            if missing_right:
                parts.append(f"missing in json-server: {missing_right[:5]}")
            return "; ".join(parts)
        for key in sorted(left_keys):
            if left[key] != right[key]:
                return f"field {key!r} differs"
        return "objects differ"

    return f"value mismatch: {left!r} != {right!r}"


def compare_results(
    *,
    scenario: Scenario,
    folder_result: FetchResult,
    json_server_result: FetchResult,
) -> dict[str, Any]:
    result = {
        "key": scenario.key,
        "label": scenario.label,
        "category": scenario.category,
        "folder_path": scenario.folder_path,
        "json_server_path": scenario.json_server_path,
        "status": "failed",
        "mismatch": "",
        "folder_count": None,
        "json_server_count": None,
    }

    if folder_result.error is not None:
        result["mismatch"] = f"dirbase {folder_result.error}"
        return result
    if json_server_result.error is not None:
        result["mismatch"] = f"json-server {json_server_result.error}"
        return result
    if folder_result.status is None or folder_result.status < 200 or folder_result.status > 299:
        result["mismatch"] = f"dirbase HTTP {folder_result.status if folder_result.status is not None else 'unknown'}"
        return result
    if json_server_result.status is None or json_server_result.status < 200 or json_server_result.status > 299:
        result["mismatch"] = (
            f"json-server HTTP {json_server_result.status if json_server_result.status is not None else 'unknown'}"
        )
        return result

    expects_paginated = has_pagination(scenario.folder_path) or has_pagination(scenario.json_server_path)
    try:
        folder_payload = normalize_payload(folder_result.payload, source="folder", expects_paginated=expects_paginated)
        json_server_payload = normalize_payload(
            json_server_result.payload,
            source="json-server",
            expects_paginated=expects_paginated,
        )
    except ValueError as exc:
        result["mismatch"] = str(exc)
        return result

    result["folder_count"] = folder_payload.count
    result["json_server_count"] = json_server_payload.count

    if type(folder_payload.value) is not type(json_server_payload.value):
        result["mismatch"] = first_difference(folder_payload.value, json_server_payload.value)
        return result

    if folder_payload.value != json_server_payload.value:
        result["mismatch"] = first_difference(folder_payload.value, json_server_payload.value)
        return result

    result["status"] = "passed"
    result["mismatch"] = ""
    return result


def run_correctness(args: argparse.Namespace) -> dict[str, Any]:
    scenarios = read_scenarios(args.scenarios_file)
    results = [
        compare_results(
            scenario=scenario,
            folder_result=fetch_json(args.folder_url, scenario.folder_path),
            json_server_result=fetch_json(args.json_server_url, scenario.json_server_path),
        )
        for scenario in scenarios
    ]
    failed = sum(1 for result in results if result["status"] != "passed")
    return {
        "status": "passed" if failed == 0 else "failed",
        "scenario_count": len(results),
        "passed": len(results) - failed,
        "failed": failed,
        "results": results,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Compare benchmark query results between dirbase and json-server.")
    parser.add_argument("--folder-url", required=True)
    parser.add_argument("--json-server-url", required=True)
    parser.add_argument("--scenarios-file", type=Path, required=True)
    parser.add_argument("--results-dir", type=Path, required=True)
    parser.add_argument("--stamp", required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    result = run_correctness(args)
    args.results_dir.mkdir(parents=True, exist_ok=True)
    output_path = args.results_dir / f"query-correctness-{args.stamp}.json"
    output_path.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(output_path)


if __name__ == "__main__":
    main()
