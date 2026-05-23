#!/usr/bin/env python3
from __future__ import annotations

import json
import tempfile
import time
import unittest
from pathlib import Path

import run_write_benchmark as bench


class WriteBenchmarkTest(unittest.TestCase):
    def test_latency_summary_sorts_and_aggregates(self) -> None:
        self.assertEqual(
            bench.latency_summary([3.0, 1.0, 2.0]),
            {"mean": 2.0, "median": 2.0, "min": 1.0, "max": 3.0},
        )

    def test_non_2xx_responses_are_counted_without_errors(self) -> None:
        started_at = time.perf_counter()
        finished_at = started_at + 2.0
        summary = bench.summarize_request_results(
            key="test",
            label="Test",
            category="write",
            started_at=started_at,
            finished_at=finished_at,
            results=[
                bench.RequestResult(0, "POST", "/members", None, 201, 4.0),
                bench.RequestResult(1, "POST", "/members", None, 404, 8.0),
            ],
        )

        self.assertEqual(summary["request_count"], 2)
        self.assertEqual(summary["successful_count"], 1)
        self.assertEqual(summary["non_2xx"], 1)
        self.assertEqual(summary["errors"], 0)
        self.assertEqual(summary["status_counts"], {"201": 1, "404": 1})
        self.assertEqual(summary["latency_ms"]["median"], 6.0)

    def test_folder_correctness_passes_for_valid_persisted_writes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.write_json(
                root / "members.json",
                [
                    {"id": 1, "username": "write-put-folder-00000"},
                    {"id": 2, "username": "existing"},
                    {
                        "id": 3,
                        "username": "existing-patched",
                        "write_patch_marker": "folder",
                        "tickets_closed": 900000,
                    },
                    {"id": 4, "username": "write-post-folder-00000"},
                ],
            )
            self.write_json(root / "write_delete_items.json", [{"id": 2}])

            result = self.sample_result(target_name="folder")
            correctness = bench.validate_correctness(
                target_name="folder",
                data_layout="folder",
                data_root=root,
                result=result,
            )

            self.assertEqual(correctness["status"], "passed")

    def test_folder_correctness_fails_for_invalid_json(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "members.json").write_text("[", encoding="utf-8")
            self.write_json(root / "write_delete_items.json", [])

            correctness = bench.validate_correctness(
                target_name="folder",
                data_layout="folder",
                data_root=root,
                result=self.sample_result(target_name="folder"),
            )

            self.assertEqual(correctness["status"], "failed")
            self.assertEqual(correctness["checks"][0]["name"], "json-parse")

    def test_folder_correctness_fails_for_wrong_row_count(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.write_json(
                root / "members.json",
                [
                    {"id": 1, "username": "write-put-folder-00000"},
                    {"id": 3, "write_patch_marker": "folder", "tickets_closed": 900000},
                ],
            )
            self.write_json(root / "write_delete_items.json", [])

            correctness = bench.validate_correctness(
                target_name="folder",
                data_layout="folder",
                data_root=root,
                result=self.sample_result(target_name="folder"),
            )

            self.assertEqual(correctness["status"], "failed")
            self.assertIn(
                "post-row-count",
                [item["name"] for item in correctness["checks"] if item["status"] == "failed"],
            )

    @staticmethod
    def write_json(path: Path, payload: object) -> None:
        path.write_text(json.dumps(payload), encoding="utf-8")

    @staticmethod
    def sample_result(target_name: str) -> dict:
        return {
            "initial_counts": {"members": 3, "write_delete_items": 2},
            "workloads": [
                {
                    "key": "post-members",
                    "successful_count": 1,
                    "successful_operations": [{"index": 0, "id": None}],
                },
                {
                    "key": "put-members",
                    "successful_count": 1,
                    "successful_operations": [{"index": 0, "id": 1}],
                },
                {
                    "key": "patch-members",
                    "successful_count": 1,
                    "successful_operations": [{"index": 0, "id": 3}],
                },
                {
                    "key": "delete-write-delete-items",
                    "successful_count": 1,
                    "successful_operations": [{"index": 0, "id": 1}],
                },
            ],
        }


if __name__ == "__main__":
    unittest.main()
