#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


def fmt_num(value: float) -> str:
    return f"{value:,.2f}"


def safe_ratio(numerator: float, denominator: float) -> str:
    if denominator == 0:
        return "n/a"
    return f"{numerator / denominator:.2f}x"


def mode_title(mode: str) -> str:
    return "With warm-up" if mode == "with_warmup" else "Without warm-up"


def render_coverage_matrix(summary: dict) -> list[str]:
    rows = summary.get("coverage_matrix") or []
    if not rows:
        write_measured = bool(summary.get("write_workloads"))
        write_status = "measured" if write_measured else "not_measured"
        rows = [
            {
                "dimension": "Read latency and throughput",
                "status": "measured",
                "evidence": "autocannon read workload results",
            },
            {
                "dimension": "Write latency",
                "status": write_status,
                "evidence": "write workload results" if write_measured else "write phase was not present in this summary",
            },
            {
                "dimension": "Persisted-write correctness",
                "status": write_status,
                "evidence": "persisted JSON checks" if write_measured else "write phase was not present in this summary",
            },
            {
                "dimension": "Cold start time",
                "status": "not_measured",
                "evidence": "not measured in this benchmark run",
            },
            {
                "dimension": "Memory usage",
                "status": "not_measured",
                "evidence": "not measured in this benchmark run",
            },
            {
                "dimension": "File watcher latency",
                "status": "not_measured",
                "evidence": "not measured in this benchmark run",
            },
            {
                "dimension": "SSE event latency",
                "status": "not_measured",
                "evidence": "not measured in this benchmark run",
            },
            {
                "dimension": "Schema inference and export time",
                "status": "not_measured",
                "evidence": "not measured in this benchmark run",
            },
            {
                "dimension": "Query correctness",
                "status": "not_measured",
                "evidence": "not measured in this benchmark run",
            },
            {
                "dimension": "Concurrent write safety",
                "status": write_status,
                "evidence": "parallel write workload results" if write_measured else "write phase was not present in this summary",
            },
        ]

    lines = [
        "## Benchmark coverage",
        "",
        "| Dimension | Status | Evidence |",
        "|---|---|---|",
    ]
    for row in rows:
        lines.append(f"| {row['dimension']} | `{row['status']}` | {row['evidence']} |")
    lines.append("")
    return lines


def render_mode(summary: dict, mode: str) -> list[str]:
    lines = [
        f"## {mode_title(mode)}",
        "",
        "| Scenario | Category | dirbase req/s | json-server req/s | json-server speedup | dirbase latency (ms) | json-server latency (ms) | dirbase slower |",
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]

    mode_results = summary["modes"][mode]
    for scenario in summary["scenarios"]:
        result = mode_results[scenario["key"]]
        folder = result["folder"]["aggregate"]
        json_server = result["json_server"]["aggregate"]
        lines.append(
            "| "
            f"`{scenario['label']}` | {scenario['category']} "
            f"| {fmt_num(folder['requests_per_sec']['median'])} "
            f"| {fmt_num(json_server['requests_per_sec']['median'])} "
            f"| {safe_ratio(json_server['requests_per_sec']['median'], folder['requests_per_sec']['median'])} "
            f"| {fmt_num(folder['latency_ms']['median'])} "
            f"| {fmt_num(json_server['latency_ms']['median'])} "
            f"| {safe_ratio(folder['latency_ms']['median'], json_server['latency_ms']['median'])} |"
        )

    lines.extend(
        [
            "",
            "| Scenario | dirbase non-2xx | dirbase errors | dirbase timeouts | json-server non-2xx | json-server errors | json-server timeouts |",
            "|---|---:|---:|---:|---:|---:|---:|",
        ]
    )

    for scenario in summary["scenarios"]:
        result = mode_results[scenario["key"]]
        folder = result["folder"]["aggregate"]
        json_server = result["json_server"]["aggregate"]
        lines.append(
            "| "
            f"`{scenario['label']}` | {folder['non_2xx']} | {folder['errors']} | {folder['timeouts']} "
            f"| {json_server['non_2xx']} | {json_server['errors']} | {json_server['timeouts']} |"
        )

    lines.append("")
    return lines


def render_write_benchmarks(summary: dict) -> list[str]:
    write_workloads = summary.get("write_workloads") or {}
    folder_workloads = {item["key"]: item for item in write_workloads.get("folder", [])}
    json_server_workloads = {
        item["key"]: item for item in write_workloads.get("json_server", [])
    }
    lines = ["## Write benchmarks", ""]

    if not folder_workloads or not json_server_workloads:
        lines.extend(["Write benchmarks were not measured in this run.", ""])
        return lines

    lines.extend(
        [
            "| Workload | Category | dirbase req/s | json-server req/s | dirbase latency (ms) | json-server latency (ms) | dirbase non-2xx | json-server non-2xx |",
            "|---|---|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for key in (
        "post-members",
        "put-members",
        "patch-members",
        "delete-write-delete-items",
    ):
        folder = folder_workloads[key]
        json_server = json_server_workloads[key]
        lines.append(
            "| "
            f"`{folder['label']}` | {folder['category']} "
            f"| {fmt_num(folder['requests_per_sec'])} "
            f"| {fmt_num(json_server['requests_per_sec'])} "
            f"| {fmt_num(folder['latency_ms']['median'])} "
            f"| {fmt_num(json_server['latency_ms']['median'])} "
            f"| {folder['non_2xx']} "
            f"| {json_server['non_2xx']} |"
        )

    lines.extend(
        [
            "",
            "### Persisted write correctness",
            "",
            "| Target | Status | Failed checks |",
            "|---|---|---:|",
        ]
    )
    for target, correctness in (summary.get("write_correctness") or {}).items():
        failed = [
            item for item in correctness.get("checks", []) if item.get("status") != "passed"
        ]
        lines.append(f"| `{target}` | `{correctness.get('status', 'unknown')}` | {len(failed)} |")

    lines.append("")
    return lines


def render_query_correctness(summary: dict) -> list[str]:
    correctness = summary.get("query_correctness") or {}
    results = correctness.get("results") or []
    lines = ["## Query correctness", ""]

    if not results:
        lines.extend(["Query correctness was not measured in this run.", ""])
        return lines

    lines.extend(
        [
            f"Status: `{correctness.get('status', 'unknown')}` "
            f"({correctness.get('passed', 0)} passed, {correctness.get('failed', 0)} failed)",
            "",
            "| Scenario | Category | Status | dirbase count | json-server count | Mismatch |",
            "|---|---|---|---:|---:|---|",
        ]
    )
    for result in results:
        folder_count = result.get("folder_count")
        json_server_count = result.get("json_server_count")
        lines.append(
            "| "
            f"`{result['label']}` | {result['category']} "
            f"| `{result.get('status', 'unknown')}` "
            f"| {folder_count if folder_count is not None else ''} "
            f"| {json_server_count if json_server_count is not None else ''} "
            f"| {result.get('mismatch') or ''} |"
        )

    lines.append("")
    return lines


def render_report(summary: dict) -> str:
    generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
    config = summary["config"]
    dataset = summary["dataset"]

    lines = [
        "# Benchmark report: dirbase vs json-server",
        "",
        f"Generated: {generated_at}",
        "",
        "## Dataset",
        "",
        f"- Dataset: `{dataset.get('dataset_name', 'synthetic')}`",
        f"- Resources: {dataset['resource_count']}",
        f"- Total rows across all resources: {dataset['total_rows']:,}",
        f"- Generated JSON folder: `{dataset['folder_dir']}`",
        f"- Generated `db.json`: `{dataset['db_path']}`",
        "",
        "### Resource sizes",
        "",
        "| Resource | Rows |",
        "|---|---:|",
    ]

    for resource in dataset.get("resources", []):
        lines.append(f"| `{resource['name']}` | {resource['rows']:,} |")

    lines.extend(
        [
            "",
            "## Run configuration",
            "",
            f"- Repeated runs per scenario: {config['runs']}",
            f"- Benchmark duration: {config['duration']}s",
            f"- Connections: {config['connections']}",
            f"- Warm-up: {config['warmup_connections']} connections for {config['warmup_duration']}s",
            f"- Write requests per method: {config.get('write_requests_per_method', 'not configured')}",
            f"- Write connections: {config.get('write_connections', 'not configured')}",
            f"- json-server version: `{config['json_server_version']}`",
            f"- Scenario count: {len(summary['scenarios'])}",
            "",
            "## Scenario set",
            "",
        ]
    )

    for scenario in summary["scenarios"]:
        if scenario["folder_path"] == scenario["json_server_path"]:
            lines.append(f"- `{scenario['label']}` ({scenario['category']}): `{scenario['folder_path']}`")
        else:
            lines.append(
                f"- `{scenario['label']}` ({scenario['category']}): "
                f"`dirbase {scenario['folder_path']}` vs `json-server {scenario['json_server_path']}`"
            )

    lines.append("")
    lines.extend(render_coverage_matrix(summary))
    lines.extend(render_query_correctness(summary))
    for mode in ("with_warmup", "without_warmup"):
        lines.extend(render_mode(summary, mode))
    lines.extend(render_write_benchmarks(summary))

    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description="Render a markdown benchmark report from summary JSON")
    parser.add_argument("--summary", required=True, help="Path to summary JSON")
    parser.add_argument("--output", required=True, help="Path to write markdown report")
    args = parser.parse_args()

    summary_path = Path(args.summary)
    output_path = Path(args.output)
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    report = render_report(summary)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(report, encoding="utf-8")


if __name__ == "__main__":
    main()
