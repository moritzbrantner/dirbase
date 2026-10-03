#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).parent))
from summarize_architecture_baseline import build_baseline

SCENARIOS = (
    "hot-read-small",
    "hot-read-window",
    "hot-read-declared-small",
    "hot-read-declared-window",
    "localized-write-small",
    "localized-write-large",
    "localized-write-ballast",
)


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value), encoding="utf-8")


class BaselineSummaryTests(unittest.TestCase):
    def make_round(
        self,
        profiler_root: Path,
        server_root: Path,
        scenario: str,
        round_number: int,
        duration: float,
        request_median: float,
        fingerprint: str = "a" * 64,
    ) -> None:
        bundle = profiler_root / scenario / f"round-{round_number}"
        write_json(
            bundle / "manifest.json",
            {
                "scenario_id": f"dirbase-{scenario}",
                "scenario_digest": (scenario[0] if scenario else "b") * 64,
                "environment_fingerprint": fingerprint,
                "source": {"git_sha": "deadbeef", "dirty": False},
            },
        )
        write_json(
            bundle / "metrics.json",
            {
                "scenario_id": f"dirbase-{scenario}",
                "samples": [
                    {"duration_ms": duration, "succeeded": True, "timed_out": False},
                    {
                        "duration_ms": duration * 1.1,
                        "succeeded": True,
                        "timed_out": False,
                    },
                ],
            },
        )
        write_json(
            server_root / scenario / f"round-{round_number}.json",
            {
                "schema": "dirbase/architecture-benchmark/v1",
                "startup_ms": 10.0 + round_number,
                "workload_ms": duration,
                "request_ms": {"median": request_median, "p95": request_median * 1.5},
                "server_peak_rss_kib": 1000 + round_number,
            },
        )

    def test_builds_paired_descriptive_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            profiler_root = root / "profiler"
            server_root = root / "server"
            scale = {
                "hot-read-small": 1.0,
                "hot-read-window": 2.0,
                "hot-read-declared-small": 1.0,
                "hot-read-declared-window": 4.0,
                "localized-write-small": 1.0,
                "localized-write-large": 2.0,
                "localized-write-ballast": 3.0,
            }
            for scenario in SCENARIOS:
                for round_number in (1, 2):
                    self.make_round(
                        profiler_root,
                        server_root,
                        scenario,
                        round_number,
                        duration=100.0 * scale[scenario],
                        request_median=5.0 * scale[scenario],
                    )

            baseline = build_baseline(profiler_root, server_root, "deadbeef", min_rounds=2)
            self.assertEqual(baseline["policy"]["state"], "descriptive")
            self.assertFalse(baseline["policy"]["thresholds_enabled"])
            self.assertEqual(baseline["scenarios"]["hot-read-small"]["round_count"], 2)
            read_ratio = baseline["amplification"]["read_source_size"]["ratios"]
            self.assertAlmostEqual(read_ratio["server_request_median_ms"]["median"], 2.0)
            validation_ratio = baseline["amplification"]["read_validation_source_size"]["ratios"]
            self.assertAlmostEqual(
                validation_ratio["server_request_median_ms"]["median"],
                4.0,
            )
            target_ratio = baseline["amplification"]["write_target_size"]["ratios"]
            self.assertAlmostEqual(target_ratio["server_request_median_ms"]["median"], 2.0)
            write_ratio = baseline["amplification"]["write_unrelated_state"]["ratios"]
            self.assertAlmostEqual(write_ratio["profiler_wall_time_ms"]["median"], 3.0)

    def test_rejects_mixed_environment_fingerprints(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            profiler_root = root / "profiler"
            server_root = root / "server"
            for scenario in SCENARIOS:
                for round_number in (1, 2):
                    fingerprint = "b" * 64 if scenario == "hot-read-window" else "a" * 64
                    self.make_round(
                        profiler_root,
                        server_root,
                        scenario,
                        round_number,
                        duration=100.0,
                        request_median=5.0,
                        fingerprint=fingerprint,
                    )

            with self.assertRaisesRegex(ValueError, "different environment fingerprints"):
                build_baseline(profiler_root, server_root, "deadbeef", min_rounds=2)


if __name__ == "__main__":
    unittest.main()
