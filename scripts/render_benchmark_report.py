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
            "| Scenario | folder non-2xx | folder errors | folder timeouts | json-server non-2xx | json-server errors | json-server timeouts |",
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
    for mode in ("with_warmup", "without_warmup"):
        lines.extend(render_mode(summary, mode))

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
