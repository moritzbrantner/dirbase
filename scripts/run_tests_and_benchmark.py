#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path


@dataclass
class StepResult:
    name: str
    command: list[str]
    status: str
    returncode: int
    duration_seconds: float
    log_path: str
    summary: str


def parse_args() -> argparse.Namespace:
    root_dir = Path(__file__).resolve().parent.parent
    default_results_dir = root_dir / "benchmarks" / "results"
    default_data_dir = root_dir / "benchmarks" / ".work" / "benchmark-data"

    parser = argparse.ArgumentParser(
        description="Run project checks, tests, benchmark data generation, and benchmarks in one go."
    )
    parser.add_argument(
        "--results-dir",
        type=Path,
        default=default_results_dir,
        help=f"Directory for suite reports and logs (default: {default_results_dir})",
    )
    parser.add_argument(
        "--data-dir",
        type=Path,
        default=default_data_dir,
        help=f"Directory for generated benchmark JSON data (default: {default_data_dir})",
    )
    parser.add_argument(
        "--force-rebuild-data",
        action="store_true",
        help="Force regeneration of the benchmark JSON data.",
    )
    parser.add_argument("--skip-fmt", action="store_true", help="Skip cargo fmt --check.")
    parser.add_argument("--skip-clippy", action="store_true", help="Skip cargo clippy.")
    parser.add_argument("--skip-tests", action="store_true", help="Skip cargo test.")
    parser.add_argument("--skip-benchmark", action="store_true", help="Skip the benchmark.")
    parser.add_argument(
        "--stop-on-failure",
        action="store_true",
        help="Stop immediately after the first failed step.",
    )
    parser.add_argument(
        "--benchmark-duration",
        type=int,
        default=10,
        help="Benchmark duration in seconds per scenario (default: 10).",
    )
    parser.add_argument(
        "--benchmark-connections",
        type=int,
        default=50,
        help="Autocannon connections for the benchmark (default: 50).",
    )
    parser.add_argument(
        "--benchmark-runs",
        type=int,
        default=3,
        help="Repeated runs per benchmark scenario (default: 3).",
    )
    parser.add_argument(
        "--benchmark-warmup-duration",
        type=int,
        default=3,
        help="Warm-up duration in seconds (default: 3).",
    )
    parser.add_argument(
        "--benchmark-warmup-connections",
        type=int,
        default=1,
        help="Warm-up connections (default: 1).",
    )
    return parser.parse_args()


def command_text(command: list[str]) -> str:
    return " ".join(shlex.quote(part) for part in command)


def run_command(
    *,
    name: str,
    command: list[str],
    root_dir: Path,
    logs_dir: Path,
    env_overrides: dict[str, str] | None = None,
) -> StepResult:
    env = os.environ.copy()
    if env_overrides:
        env.update(env_overrides)

    started = time.monotonic()
    try:
        process = subprocess.run(
            command,
            cwd=root_dir,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        returncode = process.returncode
        stdout = process.stdout
        stderr = process.stderr
    except OSError as exc:
        returncode = 127
        stdout = ""
        stderr = f"{type(exc).__name__}: {exc}\n"

    duration = time.monotonic() - started
    log_path = logs_dir / f"{name}.log"
    log_path.write_text(
        "\n".join(
            [
                f"command: {command_text(command)}",
                f"cwd: {root_dir}",
                "",
                "=== stdout ===",
                stdout,
                "",
                "=== stderr ===",
                stderr,
            ]
        ),
        encoding="utf-8",
    )

    status = "passed" if returncode == 0 else "failed"
    summary = f"exit code {returncode}"
    return StepResult(
        name=name,
        command=command,
        status=status,
        returncode=returncode,
        duration_seconds=duration,
        log_path=str(log_path),
        summary=summary,
    )


def count_named_tests(output: str) -> int:
    return sum(1 for line in output.splitlines() if line.rstrip().endswith(": test"))


def parse_generator_payload(log_path: Path) -> dict | None:
    text = log_path.read_text(encoding="utf-8")
    for line in reversed(text.splitlines()):
        stripped = line.strip()
        if not stripped:
            continue
        try:
            parsed = json.loads(stripped)
        except json.JSONDecodeError:
            continue
        if isinstance(parsed, dict) and "status" in parsed:
            return parsed
    return None


def parse_benchmark_paths(log_path: Path) -> tuple[Path | None, Path | None]:
    summary_path = None
    report_path = None
    summary_matcher = re.compile(r"^Summary:\s*(.+)$")
    report_matcher = re.compile(r"^Report:\s*(.+)$")
    for line in log_path.read_text(encoding="utf-8").splitlines():
        if summary_path is None:
            match = summary_matcher.match(line.strip())
            if match:
                summary_path = Path(match.group(1).strip())
                continue
        if report_path is None:
            match = report_matcher.match(line.strip())
            if match:
                report_path = Path(match.group(1).strip())
    return summary_path, report_path


def fmt_duration(seconds: float) -> str:
    return f"{seconds:.2f}s"


def fmt_num(value: float) -> str:
    return f"{value:,.2f}"


def safe_ratio(numerator: float, denominator: float) -> str:
    if denominator == 0:
        return "n/a"
    return f"{numerator / denominator:.2f}x"


def benchmark_mode_snapshot(summary: dict, mode: str) -> dict:
    faster = 0
    slower = 0
    tied = 0
    for scenario in summary["scenarios"]:
        result = summary["modes"][mode][scenario["key"]]
        folder = result["folder"]["aggregate"]["requests_per_sec"]["median"]
        json_server = result["json_server"]["aggregate"]["requests_per_sec"]["median"]
        if abs(folder - json_server) < 1e-9:
            tied += 1
        elif folder > json_server:
            faster += 1
        else:
            slower += 1
    return {"faster": faster, "slower": slower, "tied": tied}


def render_benchmark_table(summary: dict, mode: str) -> list[str]:
    lines = [
        (
            "| Scenario | Category | dirbase req/s | json-server req/s | json-server speedup "
            "| dirbase latency (ms) | json-server latency (ms) | dirbase slower |"
        ),
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for scenario in summary["scenarios"]:
        result = summary["modes"][mode][scenario["key"]]
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
    return lines


def render_report(
    *,
    generated_at: str,
    overall_status: str,
    steps: list[StepResult],
    test_count: int | None,
    generator_payload: dict | None,
    benchmark_summary: dict | None,
    benchmark_summary_path: Path | None,
    benchmark_report_path: Path | None,
) -> str:
    passed_steps = sum(1 for step in steps if step.status == "passed")
    failed_steps = [step for step in steps if step.status != "passed"]

    lines = [
        "# Full validation report: tests + benchmark",
        "",
        f"Generated: {generated_at}",
        "",
        "## Overall status",
        "",
        f"- Status: **{overall_status.upper()}**",
        f"- Steps passed: {passed_steps}/{len(steps)}",
    ]

    if test_count is not None:
        lines.append(f"- Named Rust tests discovered: {test_count}")

    if generator_payload is not None:
        lines.extend(
            [
                f"- Benchmark data status: `{generator_payload.get('status', 'unknown')}`",
                f"- Benchmark resources: {generator_payload.get('resource_count', 'n/a')}",
                f"- Benchmark total rows: {generator_payload.get('total_rows', 'n/a')}",
            ]
        )

    if benchmark_summary_path is not None:
        lines.append(f"- Benchmark summary JSON: `{benchmark_summary_path}`")
    if benchmark_report_path is not None:
        lines.append(f"- Benchmark markdown report: `{benchmark_report_path}`")

    lines.extend(
        [
            "",
            "## Step results",
            "",
            "| Step | Status | Duration | Command | Log |",
            "|---|---|---:|---|---|",
        ]
    )

    for step in steps:
        lines.append(
            "| "
            f"{step.name} | {step.status} | {fmt_duration(step.duration_seconds)} "
            f"| `{command_text(step.command)}` | `{step.log_path}` |"
        )

    if failed_steps:
        lines.extend(["", "## Failures", ""])
        for step in failed_steps:
            lines.append(f"- `{step.name}` failed with {step.summary}; see `{step.log_path}`.")

    if benchmark_summary is not None:
        with_warmup = benchmark_mode_snapshot(benchmark_summary, "with_warmup")
        without_warmup = benchmark_mode_snapshot(benchmark_summary, "without_warmup")
        lines.extend(
            [
                "",
                "## Benchmark highlights",
                "",
                (
                    f"- With warm-up: dirbase was faster in {with_warmup['faster']} scenarios, "
                    f"slower in {with_warmup['slower']}, tied in {with_warmup['tied']}."
                ),
                (
                    f"- Without warm-up: dirbase was faster in {without_warmup['faster']} scenarios, "
                    f"slower in {without_warmup['slower']}, tied in {without_warmup['tied']}."
                ),
                "",
                "### With warm-up",
                "",
            ]
        )
        lines.extend(render_benchmark_table(benchmark_summary, "with_warmup"))
        lines.extend(["", "### Without warm-up", ""])
        lines.extend(render_benchmark_table(benchmark_summary, "without_warmup"))

    return "\n".join(lines) + "\n"


def main() -> int:
    args = parse_args()
    root_dir = Path(__file__).resolve().parent.parent
    results_dir = args.results_dir.resolve()
    results_dir.mkdir(parents=True, exist_ok=True)

    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    logs_dir = results_dir / f"suite-logs-{stamp}"
    logs_dir.mkdir(parents=True, exist_ok=True)

    generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
    steps: list[StepResult] = []
    generator_payload: dict | None = None
    benchmark_summary: dict | None = None
    benchmark_summary_path: Path | None = None
    benchmark_report_path: Path | None = None
    test_count: int | None = None

    test_list_command = ["cargo", "test", "--", "--list"]
    if not args.skip_tests:
        test_list_step = run_command(
            name="cargo-test-list",
            command=test_list_command,
            root_dir=root_dir,
            logs_dir=logs_dir,
        )
        steps.append(test_list_step)
        if test_list_step.status == "passed":
            log_text = Path(test_list_step.log_path).read_text(encoding="utf-8")
            test_count = count_named_tests(log_text)
        elif args.stop_on_failure:
            overall_status = "failed"
            report_json = results_dir / f"suite-report-{stamp}.json"
            report_md = results_dir / f"suite-report-{stamp}.md"
            report = render_report(
                generated_at=generated_at,
                overall_status=overall_status,
                steps=steps,
                test_count=test_count,
                generator_payload=generator_payload,
                benchmark_summary=benchmark_summary,
                benchmark_summary_path=benchmark_summary_path,
                benchmark_report_path=benchmark_report_path,
            )
            report_md.write_text(report, encoding="utf-8")
            report_json.write_text(
                json.dumps(
                    {
                        "generated_at": generated_at,
                        "overall_status": overall_status,
                        "steps": [asdict(step) for step in steps],
                        "test_count": test_count,
                        "generator_payload": generator_payload,
                        "benchmark_summary_path": str(benchmark_summary_path) if benchmark_summary_path else None,
                        "benchmark_report_path": str(benchmark_report_path) if benchmark_report_path else None,
                    },
                    indent=2,
                ),
                encoding="utf-8",
            )
            print(f"Suite report: {report_md}")
            print(f"Suite JSON: {report_json}")
            return 1

    planned_steps: list[tuple[str, list[str], dict[str, str] | None]] = []

    generator_command = [
        sys.executable,
        str(root_dir / "scripts" / "build_benchmark_data.py"),
        "--output-dir",
        str(args.data_dir.resolve()),
    ]
    if args.force_rebuild_data:
        generator_command.append("--force")
    planned_steps.append(("build-benchmark-data", generator_command, None))

    if not args.skip_fmt:
        planned_steps.append(("cargo-fmt-check", ["cargo", "fmt", "--all", "--check"], None))
    if not args.skip_clippy:
        planned_steps.append(
            (
                "cargo-clippy",
                ["cargo", "clippy", "--all-targets", "--all-features", "--", "-D", "warnings"],
                None,
            )
        )
    if not args.skip_tests:
        planned_steps.append(("cargo-test", ["cargo", "test"], None))
    if not args.skip_benchmark:
        benchmark_env = {
            "DATA_DIR": str(args.data_dir.resolve()),
            "DURATION": str(args.benchmark_duration),
            "CONNECTIONS": str(args.benchmark_connections),
            "RUNS": str(args.benchmark_runs),
            "WARMUP_DURATION": str(args.benchmark_warmup_duration),
            "WARMUP_CONNECTIONS": str(args.benchmark_warmup_connections),
        }
        if args.force_rebuild_data:
            benchmark_env["FORCE_REBUILD_DATA"] = "1"
        planned_steps.append(
            (
                "benchmark-vs-json-server",
                ["bash", str(root_dir / "scripts" / "benchmark_vs_json_server.sh")],
                benchmark_env,
            )
        )

    for name, command, env_overrides in planned_steps:
        step = run_command(
            name=name,
            command=command,
            root_dir=root_dir,
            logs_dir=logs_dir,
            env_overrides=env_overrides,
        )
        steps.append(step)

        if name == "build-benchmark-data":
            generator_payload = parse_generator_payload(Path(step.log_path))
        elif name == "benchmark-vs-json-server" and step.status == "passed":
            benchmark_summary_path, benchmark_report_path = parse_benchmark_paths(Path(step.log_path))
            if benchmark_summary_path is not None and benchmark_summary_path.exists():
                benchmark_summary = json.loads(benchmark_summary_path.read_text(encoding="utf-8"))

        if step.status != "passed" and args.stop_on_failure:
            break

    overall_status = "passed" if all(step.status == "passed" for step in steps) else "failed"

    report_payload = {
        "generated_at": generated_at,
        "overall_status": overall_status,
        "steps": [asdict(step) for step in steps],
        "test_count": test_count,
        "generator_payload": generator_payload,
        "benchmark_summary_path": str(benchmark_summary_path) if benchmark_summary_path else None,
        "benchmark_report_path": str(benchmark_report_path) if benchmark_report_path else None,
        "logs_dir": str(logs_dir),
    }

    if benchmark_summary is not None:
        report_payload["benchmark_highlights"] = {
            "with_warmup": benchmark_mode_snapshot(benchmark_summary, "with_warmup"),
            "without_warmup": benchmark_mode_snapshot(benchmark_summary, "without_warmup"),
        }

    report_json = results_dir / f"suite-report-{stamp}.json"
    report_md = results_dir / f"suite-report-{stamp}.md"
    report_json.write_text(json.dumps(report_payload, indent=2), encoding="utf-8")
    report_md.write_text(
        render_report(
            generated_at=generated_at,
            overall_status=overall_status,
            steps=steps,
            test_count=test_count,
            generator_payload=generator_payload,
            benchmark_summary=benchmark_summary,
            benchmark_summary_path=benchmark_summary_path,
            benchmark_report_path=benchmark_report_path,
        ),
        encoding="utf-8",
    )

    print(f"Suite report: {report_md}")
    print(f"Suite JSON: {report_json}")
    if benchmark_report_path is not None:
        print(f"Benchmark report: {benchmark_report_path}")
    if benchmark_summary_path is not None:
        print(f"Benchmark summary: {benchmark_summary_path}")

    return 0 if overall_status == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
