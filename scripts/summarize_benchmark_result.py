#!/usr/bin/env python3
"""Print a compact, report-only summary of the latest benchmark result."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def latest_summary(root: Path) -> Path:
    candidates = sorted(root.glob("benchmarks/results/benchmark-summary-*.json"))
    if not candidates:
        raise SystemExit("No benchmark summary files found under benchmarks/results.")
    return candidates[-1]


def report_path_for(summary_path: Path) -> Path:
    stamp = summary_path.name.removeprefix("benchmark-summary-").removesuffix(".json")
    return summary_path.with_name(f"benchmark-report-{stamp}.md")


def aggregate_error_counts(summary: dict) -> dict[str, int]:
    totals = {"non_2xx": 0, "errors": 0, "timeouts": 0}
    for mode in summary.get("modes", {}).values():
        for scenario in mode.values():
            for server in ("folder", "json_server"):
                aggregate = scenario.get(server, {}).get("aggregate", {})
                for key in totals:
                    totals[key] += int(aggregate.get(key, 0) or 0)
    return totals


def slowest_dirbase_scenarios(summary: dict, limit: int) -> list[tuple[str, str, float]]:
    rows: list[tuple[str, str, float]] = []
    for mode_name, mode in summary.get("modes", {}).items():
        for scenario in mode.values():
            latency = (
                scenario.get("folder", {})
                .get("aggregate", {})
                .get("latency_ms", {})
                .get("median")
            )
            if latency is not None:
                rows.append((mode_name, scenario.get("label", "unknown"), float(latency)))
    return sorted(rows, key=lambda row: row[2], reverse=True)[:limit]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", type=Path, help="Benchmark summary JSON to summarize.")
    parser.add_argument("--limit", type=int, default=5, help="Number of slowest scenarios to show.")
    args = parser.parse_args()

    root = Path.cwd()
    summary_path = args.summary or latest_summary(root)
    summary = json.loads(summary_path.read_text())
    report_path = report_path_for(summary_path)
    scenarios = summary.get("scenarios", [])
    errors = aggregate_error_counts(summary)

    print("# Benchmark Summary")
    print()
    print(f"- Summary: `{summary_path}`")
    print(f"- Report: `{report_path}`")
    print(f"- Scenario count: {len(scenarios)}")
    print(
        "- Aggregate failures: "
        f"non-2xx={errors['non_2xx']}, errors={errors['errors']}, timeouts={errors['timeouts']}"
    )
    print("- Slowest dirbase scenarios by median latency:")
    for mode, label, latency in slowest_dirbase_scenarios(summary, args.limit):
        print(f"  - `{mode}` `{label}`: {latency:.2f} ms")


if __name__ == "__main__":
    main()
