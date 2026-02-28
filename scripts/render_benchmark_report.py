#!/usr/bin/env python3
import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


def _fmt_num(value: float) -> str:
    return f"{value:,.2f}"


def _safe_ratio(numerator: float, denominator: float) -> str:
    if denominator == 0:
        return "n/a"
    return f"{numerator / denominator:.2f}x"


def render_report(summary: dict) -> str:
    folder_item = summary["folder_item"]
    json_item = summary["json_item"]
    folder_query = summary["folder_query"]
    json_query = summary["json_query"]

    generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")

    lines = [
        "# Benchmark report: folder-server vs json-server",
        "",
        f"Generated: {generated_at}",
        "",
        "## Throughput (requests/sec)",
        "",
        "| Scenario | folder-server | json-server | json-server speedup |",
        "|---|---:|---:|---:|",
        (
            "| `GET /posts/5000` "
            f"| {_fmt_num(folder_item['requests_per_sec'])} "
            f"| {_fmt_num(json_item['requests_per_sec'])} "
            f"| {_safe_ratio(json_item['requests_per_sec'], folder_item['requests_per_sec'])} |"
        ),
        (
            "| `GET /posts?author=Author 10` "
            f"| {_fmt_num(folder_query['requests_per_sec'])} "
            f"| {_fmt_num(json_query['requests_per_sec'])} "
            f"| {_safe_ratio(json_query['requests_per_sec'], folder_query['requests_per_sec'])} |"
        ),
        "",
        "## Average latency (ms)",
        "",
        "| Scenario | folder-server | json-server | folder-server slower |",
        "|---|---:|---:|---:|",
        (
            "| `GET /posts/5000` "
            f"| {_fmt_num(folder_item['latency_ms'])} "
            f"| {_fmt_num(json_item['latency_ms'])} "
            f"| {_safe_ratio(folder_item['latency_ms'], json_item['latency_ms'])} |"
        ),
        (
            "| `GET /posts?author=Author 10` "
            f"| {_fmt_num(folder_query['latency_ms'])} "
            f"| {_fmt_num(json_query['latency_ms'])} "
            f"| {_safe_ratio(folder_query['latency_ms'], json_query['latency_ms'])} |"
        ),
        "",
        "## Reliability checks",
        "",
        "| Scenario | non-2xx | errors | timeouts |",
        "|---|---:|---:|---:|",
        (
            "| folder item | "
            f"{folder_item['non_2xx']} | {folder_item['errors']} | {folder_item['timeouts']} |"
        ),
        (
            "| json-server item | "
            f"{json_item['non_2xx']} | {json_item['errors']} | {json_item['timeouts']} |"
        ),
        (
            "| folder query | "
            f"{folder_query['non_2xx']} | {folder_query['errors']} | {folder_query['timeouts']} |"
        ),
        (
            "| json-server query | "
            f"{json_query['non_2xx']} | {json_query['errors']} | {json_query['timeouts']} |"
        ),
    ]

    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description="Render a markdown benchmark report from summary JSON")
    parser.add_argument("--summary", required=True, help="Path to summary JSON output by benchmark script")
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
