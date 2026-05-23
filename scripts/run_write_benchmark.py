#!/usr/bin/env python3
from __future__ import annotations

import argparse
import concurrent.futures
import http.client
import json
import socket
import statistics
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


SUCCESS_MIN = 200
SUCCESS_MAX = 299
REQUEST_TIMEOUT_SECONDS = 30


@dataclass(frozen=True)
class Operation:
    index: int
    method: str
    path: str
    payload: dict[str, Any] | None
    entity_id: int | None = None


@dataclass(frozen=True)
class RequestResult:
    index: int
    method: str
    path: str
    entity_id: int | None
    status: int | None
    latency_ms: float | None
    error: str | None = None
    timeout: bool = False

    @property
    def successful(self) -> bool:
        return self.status is not None and SUCCESS_MIN <= self.status <= SUCCESS_MAX


def latency_summary(values: list[float]) -> dict[str, float]:
    if not values:
        return {"mean": 0.0, "median": 0.0, "min": 0.0, "max": 0.0}
    return {
        "mean": statistics.fmean(values),
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
    }


def summarize_request_results(
    *,
    key: str,
    label: str,
    category: str,
    started_at: float,
    finished_at: float,
    results: list[RequestResult],
) -> dict[str, Any]:
    status_counts: dict[str, int] = {}
    for result in results:
        if result.status is not None:
            status_counts[str(result.status)] = status_counts.get(str(result.status), 0) + 1

    latencies = [result.latency_ms for result in results if result.latency_ms is not None]
    duration_seconds = max(finished_at - started_at, 0.0)
    successful_operations = [
        {"index": result.index, "id": result.entity_id}
        for result in results
        if result.successful
    ]

    return {
        "key": key,
        "label": label,
        "category": category,
        "request_count": len(results),
        "successful_count": len(successful_operations),
        "duration_seconds": duration_seconds,
        "requests_per_sec": (len(results) / duration_seconds) if duration_seconds > 0 else 0.0,
        "latency_ms": latency_summary([float(value) for value in latencies]),
        "status_counts": status_counts,
        "non_2xx": sum(
            1
            for result in results
            if result.status is not None and not (SUCCESS_MIN <= result.status <= SUCCESS_MAX)
        ),
        "errors": sum(1 for result in results if result.error is not None),
        "timeouts": sum(1 for result in results if result.timeout),
        "successful_operations": successful_operations,
    }


def member_payload(prefix: str, index: int, entity_id: int | None = None) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "team_id": 1,
        "username": f"{prefix}-{index:05d}",
        "role": "benchmark",
        "level": 1,
        "region": "us-east",
        "active": True,
        "remote": True,
        "tenure_years": 1.0,
        "tickets_closed": index,
    }
    if entity_id is not None:
        payload["id"] = entity_id
    return payload


def build_workloads(target_name: str, requests_per_method: int) -> list[dict[str, Any]]:
    post_operations = [
        Operation(
            index=index,
            method="POST",
            path="/members",
            payload=member_payload(f"write-post-{target_name}", index),
        )
        for index in range(requests_per_method)
    ]

    put_operations = [
        Operation(
            index=index,
            method="PUT",
            path=f"/members/{index + 1}",
            payload=member_payload(f"write-put-{target_name}", index, index + 1),
            entity_id=index + 1,
        )
        for index in range(requests_per_method)
    ]

    patch_offset = requests_per_method
    patch_operations = [
        Operation(
            index=index,
            method="PATCH",
            path=f"/members/{patch_offset + index + 1}",
            payload={
                "write_patch_marker": target_name,
                "tickets_closed": 900_000 + index,
            },
            entity_id=patch_offset + index + 1,
        )
        for index in range(requests_per_method)
    ]

    delete_operations = [
        Operation(
            index=index,
            method="DELETE",
            path=f"/write_delete_items/{index + 1}",
            payload=None,
            entity_id=index + 1,
        )
        for index in range(requests_per_method)
    ]

    return [
        {
            "key": "post-members",
            "label": "POST /members",
            "category": "write-post",
            "operations": post_operations,
        },
        {
            "key": "put-members",
            "label": "PUT /members/{id}",
            "category": "write-put",
            "operations": put_operations,
        },
        {
            "key": "patch-members",
            "label": "PATCH /members/{id}",
            "category": "write-patch",
            "operations": patch_operations,
        },
        {
            "key": "delete-write-delete-items",
            "label": "DELETE /write_delete_items/{id}",
            "category": "write-delete",
            "operations": delete_operations,
        },
    ]


def send_operation(server_url: str, operation: Operation) -> RequestResult:
    parsed = urlparse(server_url)
    if parsed.scheme != "http" or not parsed.hostname:
        raise ValueError(f"Only http server URLs are supported: {server_url}")

    port = parsed.port or 80
    base_path = parsed.path.rstrip("/")
    request_path = f"{base_path}{operation.path}"
    body = None
    headers: dict[str, str] = {}
    if operation.payload is not None:
        body = json.dumps(operation.payload, separators=(",", ":")).encode("utf-8")
        headers["content-type"] = "application/json"
        headers["content-length"] = str(len(body))

    started_at = time.perf_counter()
    connection = http.client.HTTPConnection(
        parsed.hostname,
        port,
        timeout=REQUEST_TIMEOUT_SECONDS,
    )
    try:
        connection.request(operation.method, request_path, body=body, headers=headers)
        response = connection.getresponse()
        response.read()
        latency_ms = (time.perf_counter() - started_at) * 1000
        return RequestResult(
            index=operation.index,
            method=operation.method,
            path=operation.path,
            entity_id=operation.entity_id,
            status=response.status,
            latency_ms=latency_ms,
        )
    except (TimeoutError, socket.timeout) as exc:
        return RequestResult(
            index=operation.index,
            method=operation.method,
            path=operation.path,
            entity_id=operation.entity_id,
            status=None,
            latency_ms=None,
            error=str(exc),
            timeout=True,
        )
    except OSError as exc:
        return RequestResult(
            index=operation.index,
            method=operation.method,
            path=operation.path,
            entity_id=operation.entity_id,
            status=None,
            latency_ms=None,
            error=str(exc),
        )
    finally:
        connection.close()


def run_workload(
    *,
    server_url: str,
    connections: int,
    key: str,
    label: str,
    category: str,
    operations: list[Operation],
) -> dict[str, Any]:
    started_at = time.perf_counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=connections) as executor:
        results = list(executor.map(lambda operation: send_operation(server_url, operation), operations))
    finished_at = time.perf_counter()
    return summarize_request_results(
        key=key,
        label=label,
        category=category,
        started_at=started_at,
        finished_at=finished_at,
        results=results,
    )


def load_benchmark_resources(data_layout: str, data_root: Path) -> dict[str, list[dict[str, Any]]]:
    if data_layout == "folder":
        members_path = data_root / "members.json"
        delete_path = data_root / "write_delete_items.json"
        return {
            "members": json.loads(members_path.read_text(encoding="utf-8")),
            "write_delete_items": json.loads(delete_path.read_text(encoding="utf-8")),
        }

    db = json.loads(data_root.read_text(encoding="utf-8"))
    return {
        "members": db["members"],
        "write_delete_items": db["write_delete_items"],
    }


def index_by_id(rows: list[dict[str, Any]]) -> dict[int, dict[str, Any]]:
    return {int(row["id"]): row for row in rows}


def workload_by_key(result: dict[str, Any], key: str) -> dict[str, Any]:
    for workload in result["workloads"]:
        if workload["key"] == key:
            return workload
    raise KeyError(key)


def check(condition: bool, name: str, message: str) -> dict[str, str]:
    return {"name": name, "status": "passed" if condition else "failed", "message": message}


def validate_correctness(
    *,
    target_name: str,
    data_layout: str,
    data_root: Path,
    result: dict[str, Any],
) -> dict[str, Any]:
    checks: list[dict[str, str]] = []
    try:
        resources = load_benchmark_resources(data_layout, data_root)
    except Exception as exc:  # noqa: BLE001 - report validation failure without hiding the run
        return {
            "status": "failed",
            "checks": [
                {
                    "name": "json-parse",
                    "status": "failed",
                    "message": f"failed to load persisted JSON: {exc}",
                }
            ],
        }

    members = resources["members"]
    delete_items = resources["write_delete_items"]
    members_by_id = index_by_id(members)
    delete_ids = set(index_by_id(delete_items))

    post = workload_by_key(result, "post-members")
    put = workload_by_key(result, "put-members")
    patch = workload_by_key(result, "patch-members")
    delete = workload_by_key(result, "delete-write-delete-items")

    initial_members = int(result["initial_counts"]["members"])
    initial_delete_items = int(result["initial_counts"]["write_delete_items"])
    post_rows = [
        row
        for row in members
        if isinstance(row.get("username"), str)
        and row["username"].startswith(f"write-post-{target_name}-")
    ]

    checks.append(
        check(
            len(members) == initial_members + post["successful_count"],
            "post-row-count",
            f"expected {initial_members + post['successful_count']} members, found {len(members)}",
        )
    )
    checks.append(
        check(
            len(post_rows) == post["successful_count"],
            "post-created-rows",
            f"expected {post['successful_count']} created POST rows, found {len(post_rows)}",
        )
    )

    missing_put_ids = []
    for operation in put["successful_operations"]:
        entity_id = operation["id"]
        row = members_by_id.get(entity_id)
        expected_username = f"write-put-{target_name}-{operation['index']:05d}"
        if row is None or row.get("username") != expected_username:
            missing_put_ids.append(entity_id)
    checks.append(
        check(
            not missing_put_ids,
            "put-persisted-values",
            f"PUT rows with missing replacement values: {missing_put_ids[:10]}",
        )
    )

    missing_patch_ids = []
    for operation in patch["successful_operations"]:
        entity_id = operation["id"]
        row = members_by_id.get(entity_id)
        expected_tickets_closed = 900_000 + operation["index"]
        if (
            row is None
            or row.get("write_patch_marker") != target_name
            or row.get("tickets_closed") != expected_tickets_closed
        ):
            missing_patch_ids.append(entity_id)
    checks.append(
        check(
            not missing_patch_ids,
            "patch-persisted-values",
            f"PATCH rows with missing patched values: {missing_patch_ids[:10]}",
        )
    )

    deleted_ids = {operation["id"] for operation in delete["successful_operations"]}
    remaining_deleted_ids = sorted(deleted_ids.intersection(delete_ids))
    expected_delete_count = initial_delete_items - delete["successful_count"]
    checks.append(
        check(
            len(delete_items) == expected_delete_count,
            "delete-row-count",
            f"expected {expected_delete_count} delete rows, found {len(delete_items)}",
        )
    )
    checks.append(
        check(
            not remaining_deleted_ids,
            "delete-removed-rows",
            f"deleted IDs still present: {remaining_deleted_ids[:10]}",
        )
    )

    return {
        "status": "passed" if all(item["status"] == "passed" for item in checks) else "failed",
        "checks": checks,
    }


def run_benchmark(args: argparse.Namespace) -> dict[str, Any]:
    data_root = args.data_root.resolve()
    initial_resources = load_benchmark_resources(args.data_layout, data_root)
    initial_counts = {name: len(rows) for name, rows in initial_resources.items()}
    if args.requests_per_method * 2 > initial_counts["members"]:
        raise SystemExit(
            "requests-per-method must be at most half the members row count "
            "so PUT and PATCH target distinct existing IDs"
        )
    if args.requests_per_method > initial_counts["write_delete_items"]:
        raise SystemExit(
            "requests-per-method must not exceed write_delete_items row count"
        )

    workloads = []
    for workload in build_workloads(args.target_name, args.requests_per_method):
        workloads.append(
            run_workload(
                server_url=args.server_url,
                connections=args.connections,
                key=workload["key"],
                label=workload["label"],
                category=workload["category"],
                operations=workload["operations"],
            )
        )

    result = {
        "target_name": args.target_name,
        "server_url": args.server_url,
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC"),
        "connections": args.connections,
        "requests_per_method": args.requests_per_method,
        "data_layout": args.data_layout,
        "data_root": str(data_root),
        "initial_counts": initial_counts,
        "workloads": workloads,
    }
    result["correctness"] = validate_correctness(
        target_name=args.target_name,
        data_layout=args.data_layout,
        data_root=data_root,
        result=result,
    )
    return result


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run write benchmarks against a dirbase-like JSON API.")
    parser.add_argument("--server-url", required=True)
    parser.add_argument("--target-name", required=True)
    parser.add_argument("--results-dir", type=Path, required=True)
    parser.add_argument("--stamp", required=True)
    parser.add_argument("--connections", type=int, required=True)
    parser.add_argument("--requests-per-method", type=int, required=True)
    parser.add_argument("--data-layout", choices=("folder", "json-server"), required=True)
    parser.add_argument("--data-root", type=Path, required=True)
    args = parser.parse_args()
    if args.connections < 1:
        raise SystemExit("--connections must be greater than 0")
    if args.requests_per_method < 1:
        raise SystemExit("--requests-per-method must be greater than 0")
    return args


def main() -> None:
    args = parse_args()
    result = run_benchmark(args)
    args.results_dir.mkdir(parents=True, exist_ok=True)
    output_path = args.results_dir / f"write-{args.target_name}-{args.stamp}.json"
    output_path.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(output_path)


if __name__ == "__main__":
    main()
