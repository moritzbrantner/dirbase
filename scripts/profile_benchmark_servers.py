#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class TargetMetrics:
    peak_rss_kib: int = 0
    peak_process_count: int = 0
    samples_with_processes: int = 0
    observed_pids: set[int] = field(default_factory=set)

    def observe(self, processes: list[tuple[int, int]]) -> None:
        if not processes:
            return
        self.samples_with_processes += 1
        self.peak_process_count = max(self.peak_process_count, len(processes))
        self.peak_rss_kib = max(self.peak_rss_kib, sum(rss for _, rss in processes))
        self.observed_pids.update(pid for pid, _ in processes)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a benchmark command and sample direct dirbase/json-server descendant RSS on Linux."
    )
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--interval-ms", type=int, default=50)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command and args.command[0] == "--":
        args.command = args.command[1:]
    if not args.command:
        parser.error("a command is required after --")
    if args.interval_ms < 10:
        parser.error("--interval-ms must be at least 10")
    return args


def read_process_table() -> dict[int, tuple[int, str, int]]:
    table: dict[int, tuple[int, str, int]] = {}
    proc = Path("/proc")
    if not proc.is_dir():
        return table

    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        try:
            stat = (entry / "stat").read_text(encoding="utf-8")
            close_paren = stat.rfind(")")
            fields = stat[close_paren + 2 :].split()
            ppid = int(fields[1])
            cmdline = (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode(
                "utf-8", errors="replace"
            )
            rss_kib = read_rss_kib(entry / "status")
        except (FileNotFoundError, ProcessLookupError, PermissionError, ValueError, OSError):
            continue
        table[pid] = (ppid, cmdline, rss_kib)
    return table


def read_rss_kib(status_path: Path) -> int:
    for line in status_path.read_text(encoding="utf-8").splitlines():
        if line.startswith("VmRSS:"):
            parts = line.split()
            return int(parts[1]) if len(parts) >= 2 else 0
    return 0


def descendants(root_pid: int, table: dict[int, tuple[int, str, int]]) -> set[int]:
    children: dict[int, list[int]] = {}
    for pid, (ppid, _, _) in table.items():
        children.setdefault(ppid, []).append(pid)

    found: set[int] = set()
    pending = [root_pid]
    while pending:
        parent = pending.pop()
        for child in children.get(parent, []):
            if child in found:
                continue
            found.add(child)
            pending.append(child)
    return found


def classify(cmdline: str) -> str | None:
    normalized = cmdline.replace("\\", "/")
    if "target/release/dirbase" in normalized or (
        normalized.rstrip().endswith("/dirbase") and "--folder" in normalized
    ):
        return "dirbase"
    if "json-server" in normalized:
        return "json_server"
    return None


def main() -> int:
    args = parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)

    started = time.monotonic()
    process = subprocess.Popen(args.command, start_new_session=True)
    metrics = {"dirbase": TargetMetrics(), "json_server": TargetMetrics()}
    supported = Path("/proc").is_dir()

    try:
        while process.poll() is None:
            if supported:
                table = read_process_table()
                descendant_pids = descendants(process.pid, table)
                by_target: dict[str, list[tuple[int, int]]] = {
                    "dirbase": [],
                    "json_server": [],
                }
                for pid in descendant_pids:
                    process_info = table.get(pid)
                    if process_info is None:
                        continue
                    _, cmdline, rss_kib = process_info
                    target = classify(cmdline)
                    if target is not None:
                        by_target[target].append((pid, rss_kib))
                for target, observed in by_target.items():
                    metrics[target].observe(observed)
            time.sleep(args.interval_ms / 1000)
    except KeyboardInterrupt:
        process.terminate()
        process.wait(timeout=10)
        raise

    exit_code = process.wait()
    duration_ms = round((time.monotonic() - started) * 1000)
    payload = {
        "schema_version": "dirbase/server-process-metrics/v1",
        "supported": supported,
        "platform": sys.platform,
        "sample_interval_ms": args.interval_ms,
        "duration_ms": duration_ms,
        "exit_code": exit_code,
        "command": args.command,
        "targets": {
            target: {
                "peak_process_tree_rss_kib": value.peak_rss_kib,
                "peak_process_count": value.peak_process_count,
                "samples_with_processes": value.samples_with_processes,
                "observed_pids": sorted(value.observed_pids),
            }
            for target, value in metrics.items()
        },
    }
    args.output.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    if supported and exit_code == 0:
        missing = [
            target
            for target, value in metrics.items()
            if value.samples_with_processes == 0
        ]
        if missing:
            print(
                f"server-process evidence incomplete: no samples observed for {', '.join(missing)}",
                file=sys.stderr,
            )
            return 3

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
