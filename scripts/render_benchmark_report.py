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


def _mode_title(mode: str) -> str:
    return "With warm-up" if mode == "with_warmup" else "Without warm-up"


def _render_mode(lines: list[str], mode: str, mode_summary: dict) -> None:
    folder_item = mode_summary["folder_item"]["aggregate"]
    json_item = mode_summary["json_item"]["aggregate"]
    folder_query = mode_summary["folder_query"]["aggregate"]
    json_query = mode_summary["json_query"]["aggregate"]

    lines.extend(
        [
            f"### {_mode_title(mode)}",
            "",
            "Throughput and latency tables use median values across repeated runs.",
            "",
            "#### Throughput (requests/sec, median)",
            "",
            "| Scenario | folder-server | json-server | json-server speedup |",
            "|---|---:|---:|---:|",
            (
                "| `GET /posts/5000` "
                f"| {_fmt_num(folder_item['requests_per_sec']['median'])} "
                f"| {_fmt_num(json_item['requests_per_sec']['median'])} "
                f"| {_safe_ratio(json_item['requests_per_sec']['median'], folder_item['requests_per_sec']['median'])} |"
            ),
            (
                "| `GET /posts?author=Author 10` "
                f"| {_fmt_num(folder_query['requests_per_sec']['median'])} "
                f"| {_fmt_num(json_query['requests_per_sec']['median'])} "
                f"| {_safe_ratio(json_query['requests_per_sec']['median'], folder_query['requests_per_sec']['median'])} |"
            ),
            "",
            "#### Average latency (ms, median)",
            "",
            "| Scenario | folder-server | json-server | folder-server slower |",
            "|---|---:|---:|---:|",
            (
                "| `GET /posts/5000` "
                f"| {_fmt_num(folder_item['latency_ms']['median'])} "
                f"| {_fmt_num(json_item['latency_ms']['median'])} "
                f"| {_safe_ratio(folder_item['latency_ms']['median'], json_item['latency_ms']['median'])} |"
            ),
            (
                "| `GET /posts?author=Author 10` "
                f"| {_fmt_num(folder_query['latency_ms']['median'])} "
                f"| {_fmt_num(json_query['latency_ms']['median'])} "
                f"| {_safe_ratio(folder_query['latency_ms']['median'], json_query['latency_ms']['median'])} |"
            ),
            "",
            "#### Reliability checks (sum across runs)",
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
            "",
        ]
    )


def render_report(summary: dict) -> str:
    generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
    config = summary["config"]

    lines = [
        "# Benchmark report: folder-server vs json-server",
        "",
        f"Generated: {generated_at}",
        "",
        "## Run configuration",
        "",
        f"- Repeated runs per scenario: {config['runs']}",
        f"- Benchmark duration: {config['duration']}s",
        f"- Connections: {config['connections']}",
        f"- Warm-up: {config['warmup_connections']} connections for {config['warmup_duration']}s",
        f"- Dataset size: {config['amount']}",
        "",
        "## Results",
        "",
    ]

    for mode in ("with_warmup", "without_warmup"):
        _render_mode(lines, mode, summary["modes"][mode])

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
