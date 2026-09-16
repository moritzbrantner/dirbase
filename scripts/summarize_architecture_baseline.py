#!/usr/bin/env python3
"""Aggregate repeated dirbase architecture evidence into a descriptive baseline."""

from __future__ import annotations

import argparse
import json
import math
import statistics
from pathlib import Path
from typing import Any

SCENARIOS = (
    "hot-read-small",
    "hot-read-window",
    "localized-write-small",
    "localized-write-ballast",
)
AMPLIFICATION_PAIRS = (
    (
        "read_source_size",
        "hot-read-small",
        "hot-read-window",
        "8k -> 48k source rows with the same 8-row result window",
    ),
    (
        "write_unrelated_state",
        "localized-write-small",
        "localized-write-ballast",
        "same 8k target with 80k unrelated rows added",
    ),
)


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def p95(values: list[float]) -> float:
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, math.ceil(len(ordered) * 0.95) - 1))
    return ordered[index]


def summarize(values: list[float]) -> dict[str, float | int | None]:
    if not values:
        raise ValueError("cannot summarize an empty sample set")
    mean = statistics.fmean(values)
    deviation = statistics.pstdev(values) if len(values) > 1 else 0.0
    return {
        "sample_count": len(values),
        "minimum": round(min(values), 6),
        "maximum": round(max(values), 6),
        "mean": round(mean, 6),
        "median": round(statistics.median(values), 6),
        "p95": round(p95(values), 6),
        "standard_deviation": round(deviation, 6),
        "coefficient_of_variation": None if mean == 0 else round(deviation / abs(mean), 6),
    }


def round_direct_evidence(server_root: Path, scenario: str) -> dict[int, dict[str, Any]]:
    scenario_root = server_root / scenario
    if not scenario_root.is_dir():
        raise ValueError(f"missing server evidence directory for {scenario}: {scenario_root}")
    records: dict[int, dict[str, Any]] = {}
    for path in sorted(scenario_root.glob("round-*.json")):
        try:
            round_number = int(path.stem.removeprefix("round-"))
        except ValueError as error:
            raise ValueError(f"invalid evidence round filename: {path.name}") from error
        record = load_json(path)
        if record.get("schema") != "dirbase/architecture-benchmark/v1":
            raise ValueError(f"unexpected architecture evidence schema in {path}")
        records[round_number] = record
    return records


def round_profiler_evidence(profiler_root: Path, scenario: str) -> dict[int, dict[str, Any]]:
    scenario_root = profiler_root / scenario
    if not scenario_root.is_dir():
        raise ValueError(f"missing Runtime Profiler directory for {scenario}: {scenario_root}")
    records: dict[int, dict[str, Any]] = {}
    for bundle in sorted(scenario_root.glob("round-*")):
        if not bundle.is_dir():
            continue
        try:
            round_number = int(bundle.name.removeprefix("round-"))
        except ValueError as error:
            raise ValueError(f"invalid profiler round directory: {bundle.name}") from error
        manifest = load_json(bundle / "manifest.json")
        metrics = load_json(bundle / "metrics.json")
        records[round_number] = {"manifest": manifest, "metrics": metrics}
    return records


def validate_profiler_round(
    scenario: str, round_number: int, record: dict[str, Any]
) -> tuple[list[float], str, str, str | None]:
    manifest = record["manifest"]
    metrics = record["metrics"]
    expected_id = f"dirbase-{scenario}"
    if manifest.get("scenario_id") != expected_id or metrics.get("scenario_id") != expected_id:
        raise ValueError(
            f"scenario identity mismatch for {scenario} round {round_number}: "
            f"manifest={manifest.get('scenario_id')!r}, metrics={metrics.get('scenario_id')!r}"
        )
    samples = metrics.get("samples") or []
    if not samples:
        raise ValueError(f"no Runtime Profiler samples for {scenario} round {round_number}")
    failed = [sample for sample in samples if not sample.get("succeeded") or sample.get("timed_out")]
    if failed:
        raise ValueError(f"failed Runtime Profiler samples for {scenario} round {round_number}")
    durations = [float(sample["duration_ms"]) for sample in samples]
    return (
        durations,
        str(manifest["scenario_digest"]),
        str(manifest["environment_fingerprint"]),
        manifest.get("source", {}).get("git_sha"),
    )


def scenario_summary(
    profiler_root: Path,
    server_root: Path,
    scenario: str,
    min_rounds: int,
) -> tuple[dict[str, Any], dict[int, dict[str, float]]]:
    profiler = round_profiler_evidence(profiler_root, scenario)
    direct = round_direct_evidence(server_root, scenario)
    common_rounds = sorted(set(profiler) & set(direct))
    if len(common_rounds) < min_rounds:
        raise ValueError(
            f"{scenario} has {len(common_rounds)} complete rounds; need at least {min_rounds}"
        )
    if set(profiler) != set(direct):
        raise ValueError(
            f"{scenario} has unpaired evidence: profiler={sorted(profiler)}, server={sorted(direct)}"
        )

    wall_times: list[float] = []
    scenario_digests: set[str] = set()
    fingerprints: set[str] = set()
    source_revisions: set[str] = set()
    startup_ms: list[float] = []
    workload_ms: list[float] = []
    request_median_ms: list[float] = []
    request_p95_ms: list[float] = []
    peak_rss_kib: list[float] = []
    per_round: dict[int, dict[str, float]] = {}

    for round_number in common_rounds:
        durations, digest, fingerprint, source_revision = validate_profiler_round(
            scenario, round_number, profiler[round_number]
        )
        wall_times.extend(durations)
        scenario_digests.add(digest)
        fingerprints.add(fingerprint)
        if source_revision:
            source_revisions.add(str(source_revision))

        evidence = direct[round_number]
        server_rss = evidence.get("server_peak_rss_kib")
        if server_rss is None:
            raise ValueError(f"server RSS unavailable for {scenario} round {round_number}")
        startup = float(evidence["startup_ms"])
        workload = float(evidence["workload_ms"])
        request_median = float(evidence["request_ms"]["median"])
        request_p95 = float(evidence["request_ms"]["p95"])
        rss = float(server_rss)
        startup_ms.append(startup)
        workload_ms.append(workload)
        request_median_ms.append(request_median)
        request_p95_ms.append(request_p95)
        peak_rss_kib.append(rss)
        per_round[round_number] = {
            "profiler_wall_time_ms": statistics.median(durations),
            "server_startup_ms": startup,
            "server_workload_ms": workload,
            "server_request_median_ms": request_median,
            "server_request_p95_ms": request_p95,
            "server_peak_rss_kib": rss,
        }

    if len(scenario_digests) != 1:
        raise ValueError(f"{scenario} changed Runtime Profiler scenario identity across rounds")
    if len(fingerprints) != 1:
        raise ValueError(f"{scenario} changed environment fingerprint across rounds")
    if len(source_revisions) > 1:
        raise ValueError(f"{scenario} changed source revision across rounds")

    summary = {
        "scenario_id": f"dirbase-{scenario}",
        "scenario_digest": next(iter(scenario_digests)),
        "environment_fingerprint": next(iter(fingerprints)),
        "source_revision": next(iter(source_revisions)) if source_revisions else None,
        "round_count": len(common_rounds),
        "profiler": {"wall_time_ms": summarize(wall_times)},
        "server": {
            "startup_ms": summarize(startup_ms),
            "workload_ms": summarize(workload_ms),
            "request_median_ms": summarize(request_median_ms),
            "request_p95_ms": summarize(request_p95_ms),
            "peak_rss_kib": summarize(peak_rss_kib),
        },
    }
    return summary, per_round


def ratio(expanded: float, reference: float) -> float:
    if reference == 0:
        raise ValueError("cannot calculate amplification from a zero reference")
    return expanded / reference


def amplification_summary(per_round: dict[str, dict[int, dict[str, float]]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    metrics = (
        "profiler_wall_time_ms",
        "server_startup_ms",
        "server_workload_ms",
        "server_request_median_ms",
        "server_request_p95_ms",
        "server_peak_rss_kib",
    )
    for key, reference_name, expanded_name, description in AMPLIFICATION_PAIRS:
        reference = per_round[reference_name]
        expanded = per_round[expanded_name]
        rounds = sorted(set(reference) & set(expanded))
        if not rounds:
            raise ValueError(f"no paired rounds for amplification pair {key}")
        output[key] = {
            "description": description,
            "reference": reference_name,
            "expanded": expanded_name,
            "round_count": len(rounds),
            "ratios": {
                metric: summarize(
                    [ratio(expanded[number][metric], reference[number][metric]) for number in rounds]
                )
                for metric in metrics
            },
        }
    return output


def build_baseline(
    profiler_root: Path,
    server_root: Path,
    source_revision: str,
    min_rounds: int,
) -> dict[str, Any]:
    scenarios: dict[str, Any] = {}
    per_round: dict[str, dict[int, dict[str, float]]] = {}
    fingerprints: set[str] = set()
    observed_source_revisions: set[str] = set()

    for scenario in SCENARIOS:
        summary, rounds = scenario_summary(profiler_root, server_root, scenario, min_rounds)
        scenarios[scenario] = summary
        per_round[scenario] = rounds
        fingerprints.add(summary["environment_fingerprint"])
        if summary["source_revision"]:
            observed_source_revisions.add(summary["source_revision"])

    if len(fingerprints) != 1:
        raise ValueError("architecture scenarios were captured on different environment fingerprints")
    if len(observed_source_revisions) > 1:
        raise ValueError("architecture scenarios were captured from different source revisions")
    if observed_source_revisions and source_revision not in observed_source_revisions:
        raise ValueError(
            f"requested source revision {source_revision} does not match profiler source "
            f"{next(iter(observed_source_revisions))}"
        )

    return {
        "schema": "dirbase/architecture-baseline/v1",
        "source_revision": source_revision,
        "environment_fingerprint": next(iter(fingerprints)),
        "minimum_rounds": min_rounds,
        "policy": {
            "state": "descriptive",
            "thresholds_enabled": False,
            "promotion_rule": (
                "Use this artifact as the reference evidence for the same scenario digests and "
                "environment fingerprint. Introduce budgets only after repeated baseline runs show "
                "stable variance."
            ),
        },
        "scenarios": scenarios,
        "amplification": amplification_summary(per_round),
    }


def render_markdown(baseline: dict[str, Any]) -> str:
    lines = [
        "# Dirbase architecture baseline",
        "",
        f"Source revision: `{baseline['source_revision']}`",
        f"Environment fingerprint: `{baseline['environment_fingerprint']}`",
        "",
        "This baseline is descriptive evidence; no performance threshold is enabled yet.",
        "",
        "## Scenario medians",
        "",
        "| scenario | rounds | profiler wall time | request median | request p95 | server peak RSS |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for scenario in SCENARIOS:
        summary = baseline["scenarios"][scenario]
        lines.append(
            "| {scenario} | {rounds} | {wall:.3f} ms | {median:.3f} ms | {p95_value:.3f} ms | {rss:.0f} KiB |".format(
                scenario=scenario,
                rounds=summary["round_count"],
                wall=summary["profiler"]["wall_time_ms"]["median"],
                median=summary["server"]["request_median_ms"]["median"],
                p95_value=summary["server"]["request_p95_ms"]["median"],
                rss=summary["server"]["peak_rss_kib"]["median"],
            )
        )

    lines.extend(["", "## Architectural amplification", ""])
    for key, *_ in AMPLIFICATION_PAIRS:
        pair = baseline["amplification"][key]
        lines.extend(
            [
                f"### {key}",
                "",
                pair["description"],
                "",
                "| metric | median amplification | p95 amplification | variation (CV) |",
                "| --- | ---: | ---: | ---: |",
            ]
        )
        for metric, stats in pair["ratios"].items():
            cv = stats["coefficient_of_variation"]
            cv_text = "n/a" if cv is None else f"{cv:.3f}"
            lines.append(
                f"| {metric} | {stats['median']:.3f}x | {stats['p95']:.3f}x | {cv_text} |"
            )
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--profiler-root", type=Path, required=True)
    parser.add_argument("--server-root", type=Path, required=True)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--min-rounds", type=int, default=3)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--markdown-output", type=Path)
    args = parser.parse_args()

    if args.min_rounds < 2:
        raise SystemExit("--min-rounds must be at least 2")
    baseline = build_baseline(
        args.profiler_root, args.server_root, args.source_revision, args.min_rounds
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(baseline, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown_output:
        args.markdown_output.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_output.write_text(render_markdown(baseline), encoding="utf-8")
    print(json.dumps(baseline, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
