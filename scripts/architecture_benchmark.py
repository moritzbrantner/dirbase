#!/usr/bin/env python3
"""Architecture-focused correctness and performance workloads for dirbase.

The harness keeps fixture generation outside measured profiler runs, starts a
release dirbase process, drives a bounded workload, and records server-specific
runtime evidence. Moonlight uses the deterministic `contract` output; Runtime
Profiler wraps `run` commands and owns cross-revision runtime evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import statistics
import subprocess
import threading
import time
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parent.parent
WORK_ROOT = ROOT / "benchmarks" / ".work" / "architecture"

FIXTURES = {
    "read-large": {"target_rows": 48_000, "ballast_resources": 0, "ballast_rows": 0},
    "read-declared-small": {
        "target_rows": 8_000,
        "ballast_resources": 0,
        "ballast_rows": 0,
        "declared_schema": True,
    },
    "read-declared-large": {
        "target_rows": 48_000,
        "ballast_resources": 0,
        "ballast_rows": 0,
        "declared_schema": True,
    },
    "write-small": {"target_rows": 8_000, "ballast_resources": 0, "ballast_rows": 0},
    "write-large": {"target_rows": 48_000, "ballast_resources": 0, "ballast_rows": 0},
    "write-ballast": {
        "target_rows": 8_000,
        "ballast_resources": 4,
        "ballast_rows": 20_000,
    },
}


def json_bytes(value: Any) -> bytes:
    return json.dumps(value, separators=(",", ":"), sort_keys=True).encode("utf-8")


def resource_rows(count: int, namespace: str) -> list[dict[str, Any]]:
    return [
        {
            "id": index + 1,
            "group": index % 32,
            "active": index % 2 == 0,
            "value": index,
            "label": f"{namespace}-{index % 128:03d}",
        }
        for index in range(count)
    ]


def write_json(path: Path, value: Any) -> None:
    path.write_bytes(json_bytes(value) + b"\n")


def prepare_fixture(
    name: str,
    target_rows: int,
    ballast_resources: int,
    ballast_rows: int,
    declared_schema: bool = False,
) -> Path:
    fixture = WORK_ROOT / name
    if fixture.exists():
        shutil.rmtree(fixture)
    fixture.mkdir(parents=True)
    write_json(fixture / "target.json", resource_rows(target_rows, "target"))
    if declared_schema:
        write_json(
            fixture / "schema.json",
            {
                "tables": {
                    "target": {
                        "kind": "object",
                        "primary_key": "id",
                        "columns": {
                            "id": {"column_type": "integer", "nullable": False},
                            "group": {"column_type": "integer", "nullable": False},
                            "active": {"column_type": "boolean", "nullable": False},
                            "value": {"column_type": "integer", "nullable": False},
                            "label": {"column_type": "string", "nullable": False},
                        },
                        "foreign_keys": {},
                    }
                }
            },
        )
    for index in range(ballast_resources):
        write_json(
            fixture / f"ballast_{index:02d}.json",
            resource_rows(ballast_rows, f"ballast-{index:02d}"),
        )
    return fixture


def prepare_all() -> None:
    for name, config in FIXTURES.items():
        prepare_fixture(name, **config)
    print(json.dumps({"prepared": sorted(FIXTURES)}, separators=(",", ":"), sort_keys=True))


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def request_json(base_url: str, method: str, path: str, body: Any | None = None) -> Any:
    data = None if body is None else json_bytes(body)
    request = Request(
        f"{base_url}{path}",
        data=data,
        method=method,
        headers={"content-type": "application/json"} if data is not None else {},
    )
    try:
        with urlopen(request, timeout=15) as response:
            payload = response.read()
    except HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{method} {path} returned HTTP {error.code}: {detail}") from error
    except URLError as error:
        raise RuntimeError(f"{method} {path} failed: {error}") from error
    return json.loads(payload)


def wait_until_ready(base_url: str, process: subprocess.Popen[bytes], timeout_seconds: float = 20.0) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"dirbase exited before becoming ready with status {process.returncode}")
        try:
            with urlopen(f"{base_url}/healthz", timeout=1) as response:
                if response.status == 200:
                    return
        except (HTTPError, URLError, TimeoutError) as error:
            last_error = error
        time.sleep(0.025)
    raise RuntimeError(f"dirbase did not become ready: {last_error}")


def process_rss_kib(pid: int) -> int | None:
    try:
        for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8").splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1])
    except (FileNotFoundError, PermissionError, ValueError):
        return None
    return None


class RssSampler:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.peak_kib: int | None = None
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self) -> None:
        while not self._stop.is_set():
            sample = process_rss_kib(self.pid)
            if sample is not None:
                self.peak_kib = sample if self.peak_kib is None else max(self.peak_kib, sample)
            self._stop.wait(0.01)

    def __enter__(self) -> "RssSampler":
        self._thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self._stop.set()
        self._thread.join(timeout=1)


def percentile(samples: list[float], fraction: float) -> float:
    if not samples:
        return 0.0
    ordered = sorted(samples)
    index = min(len(ordered) - 1, max(0, int(round((len(ordered) - 1) * fraction))))
    return ordered[index]


def file_digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def normalize_page(value: Any) -> list[dict[str, Any]]:
    if isinstance(value, list):
        return value
    if isinstance(value, dict) and isinstance(value.get("data"), list):
        return value["data"]
    raise RuntimeError(f"unexpected paginated response shape: {type(value).__name__}")


def start_server(binary: Path, fixture: Path) -> tuple[subprocess.Popen[bytes], str, float]:
    port = free_port()
    base_url = f"http://127.0.0.1:{port}"
    started = time.perf_counter()
    process = subprocess.Popen(
        [str(binary), "--folder", str(fixture), "--bind", f"127.0.0.1:{port}"],
        cwd=fixture,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        wait_until_ready(base_url, process)
    except Exception:
        stop_server(process)
        raise
    return process, base_url, (time.perf_counter() - started) * 1000.0


def stop_server(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=3)


def hot_read(base_url: str, iterations: int) -> tuple[list[float], dict[str, Any]]:
    request_json(base_url, "GET", "/target/1")
    samples: list[float] = []
    last_page: list[dict[str, Any]] = []
    for index in range(iterations):
        group = index % 32
        started = time.perf_counter()
        response = request_json(
            base_url,
            "GET",
            f"/target?group:eq={group}&_sort=-value&_page=1&_per_page=8",
        )
        samples.append((time.perf_counter() - started) * 1000.0)
        last_page = normalize_page(response)
        if len(last_page) != 8 or any(row.get("group") != group for row in last_page):
            raise RuntimeError("hot-read response violated filter/page contract")
    return samples, {
        "last_page_ids": [row["id"] for row in last_page],
        "page_size": len(last_page),
    }


def localized_write(base_url: str, fixture: Path, iterations: int) -> tuple[list[float], dict[str, Any]]:
    request_json(base_url, "GET", "/target/1")
    ballast_paths = sorted(fixture.glob("ballast_*.json"))
    before = {path.name: file_digest(path) for path in ballast_paths}
    samples: list[float] = []
    final_value = None
    for index in range(iterations):
        final_value = 1_000_000 + index
        started = time.perf_counter()
        response = request_json(base_url, "PATCH", "/target/1", {"value": final_value})
        samples.append((time.perf_counter() - started) * 1000.0)
        if response.get("value") != final_value:
            raise RuntimeError("PATCH response did not expose the applied delta")
    after = {path.name: file_digest(path) for path in ballast_paths}
    target = json.loads((fixture / "target.json").read_text(encoding="utf-8"))
    if len(target) == 0 or target[0].get("value") != final_value:
        raise RuntimeError("persisted target resource does not match the final mutation")
    if before != after:
        raise RuntimeError("localized mutation changed unrelated resource bytes")
    return samples, {
        "ballast_resources_unchanged": len(ballast_paths),
        "final_value": final_value,
        "row_count": len(target),
    }


def run_workload(binary: Path, fixture_name: str, scenario: str, iterations: int) -> dict[str, Any]:
    fixture = WORK_ROOT / fixture_name
    if not (fixture / "target.json").exists():
        raise RuntimeError(f"fixture is not prepared: {fixture_name}; run prepare-all first")
    process, base_url, startup_ms = start_server(binary, fixture)
    workload_ms = 0.0
    try:
        started = time.perf_counter()
        with RssSampler(process.pid) as rss:
            if scenario == "hot-read":
                samples, semantic = hot_read(base_url, iterations)
            elif scenario == "localized-write":
                samples, semantic = localized_write(base_url, fixture, iterations)
            else:
                raise RuntimeError(f"unsupported scenario: {scenario}")
        workload_ms = (time.perf_counter() - started) * 1000.0
        peak_rss_kib = rss.peak_kib
    finally:
        stop_server(process)
    return {
        "schema": "dirbase/architecture-benchmark/v1",
        "scenario": scenario,
        "fixture": fixture_name,
        "iterations": iterations,
        "startup_ms": round(startup_ms, 3),
        "workload_ms": round(workload_ms, 3),
        "request_ms": {
            "median": round(statistics.median(samples), 3),
            "p95": round(percentile(samples, 0.95), 3),
            "max": round(max(samples), 3),
        },
        "server_peak_rss_kib": peak_rss_kib,
        "semantic": semantic,
    }


def contract(binary: Path) -> None:
    name = f"contract-{os.getpid()}"
    fixture = prepare_fixture(name, target_rows=256, ballast_resources=2, ballast_rows=128)
    try:
        process, base_url, _ = start_server(binary, fixture)
        try:
            _, read_summary = hot_read(base_url, 3)
            _, write_summary = localized_write(base_url, fixture, 3)
            persisted = request_json(base_url, "GET", "/target/1")
        finally:
            stop_server(process)
        output = {
            "schema": "dirbase/architecture-contract/v1",
            "hot_read": read_summary,
            "localized_write": write_summary,
            "persisted_id": persisted.get("id"),
            "persisted_value": persisted.get("value"),
        }
        print(json.dumps(output, separators=(",", ":"), sort_keys=True))
    finally:
        shutil.rmtree(fixture, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("prepare-all")

    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--scenario", choices=["hot-read", "localized-write"], required=True)
    run_parser.add_argument("--fixture", choices=sorted(FIXTURES), required=True)
    run_parser.add_argument("--iterations", type=int, default=20)
    run_parser.add_argument("--binary", type=Path, default=ROOT / "target" / "release" / "dirbase")
    run_parser.add_argument("--output", type=Path, required=True)

    contract_parser = subparsers.add_parser("contract")
    contract_parser.add_argument("--binary", type=Path, default=ROOT / "target" / "release" / "dirbase")

    args = parser.parse_args()
    if args.command == "prepare-all":
        prepare_all()
        return 0

    binary = args.binary.resolve()
    if not binary.exists():
        raise SystemExit(f"dirbase release binary does not exist: {binary}")

    if args.command == "contract":
        contract(binary)
        return 0

    if args.iterations < 1:
        raise SystemExit("--iterations must be at least 1")
    result = run_workload(binary, args.fixture, args.scenario, args.iterations)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
