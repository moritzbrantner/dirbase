#!/usr/bin/env python3
from __future__ import annotations

import unittest

import run_query_correctness as correctness


class QueryCorrectnessTest(unittest.TestCase):
    def test_normalize_plain_object(self) -> None:
        payload = {"id": 1, "name": "Ada"}

        normalized = correctness.normalize_payload(payload, source="folder", expects_paginated=False)

        self.assertEqual(normalized.value, payload)
        self.assertIsNone(normalized.count)

    def test_normalize_plain_array(self) -> None:
        payload = [{"id": 1}, {"id": 2}]

        normalized = correctness.normalize_payload(payload, source="json-server", expects_paginated=False)

        self.assertEqual(normalized.value, payload)
        self.assertEqual(normalized.count, 2)

    def test_normalize_dirbase_paginated_response_uses_data(self) -> None:
        payload = {"first": 1, "next": None, "items": 1, "data": [{"id": 1}]}

        normalized = correctness.normalize_payload(payload, source="folder", expects_paginated=True)

        self.assertEqual(normalized.value, [{"id": 1}])
        self.assertEqual(normalized.count, 1)

    def test_normalize_malformed_dirbase_paginated_response_fails(self) -> None:
        with self.assertRaisesRegex(ValueError, "missing a data array"):
            correctness.normalize_payload({"items": 0}, source="folder", expects_paginated=True)

    def test_identical_values_pass(self) -> None:
        result = correctness.compare_results(
            scenario=self.scenario(),
            folder_result=correctness.FetchResult(200, [{"id": 1}]),
            json_server_result=correctness.FetchResult(200, [{"id": 1}]),
        )

        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["folder_count"], 1)
        self.assertEqual(result["json_server_count"], 1)

    def test_different_lengths_fail(self) -> None:
        result = correctness.compare_results(
            scenario=self.scenario(),
            folder_result=correctness.FetchResult(200, [{"id": 1}]),
            json_server_result=correctness.FetchResult(200, [{"id": 1}, {"id": 2}]),
        )

        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["mismatch"], "array length mismatch: 1 != 2")

    def test_different_items_fail(self) -> None:
        result = correctness.compare_results(
            scenario=self.scenario(),
            folder_result=correctness.FetchResult(200, [{"id": 1}]),
            json_server_result=correctness.FetchResult(200, [{"id": 2}]),
        )

        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["mismatch"], "array item 0 differs")

    def test_non_2xx_response_fails(self) -> None:
        result = correctness.compare_results(
            scenario=self.scenario(),
            folder_result=correctness.FetchResult(404, None),
            json_server_result=correctness.FetchResult(200, []),
        )

        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["mismatch"], "dirbase HTTP 404")

    def test_paginated_dirbase_compares_data_against_json_server_array(self) -> None:
        result = correctness.compare_results(
            scenario=self.scenario(
                folder_path="/members?_page=1&_per_page=1",
                json_server_path="/members?_page=1&_limit=1",
            ),
            folder_result=correctness.FetchResult(200, {"data": [{"id": 1}], "items": 2}),
            json_server_result=correctness.FetchResult(200, [{"id": 1}]),
        )

        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["folder_count"], 1)
        self.assertEqual(result["json_server_count"], 1)

    @staticmethod
    def scenario(folder_path: str = "/members", json_server_path: str = "/members") -> correctness.Scenario:
        return correctness.Scenario(
            key="members",
            label="Members",
            folder_path=folder_path,
            json_server_path=json_server_path,
            category="filter",
        )


if __name__ == "__main__":
    unittest.main()
