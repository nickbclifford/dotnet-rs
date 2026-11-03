#!/usr/bin/env python3
"""Exercise the actual CI aggregate shell blocks with simulated job results."""

import json
import os
from pathlib import Path
import re
import subprocess
import textwrap
import unittest

WORKFLOW = (Path(__file__).resolve().parents[2] / ".github/workflows/ci.yml").read_text()


def job(name):
    match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:\n|\Z)", WORKFLOW, re.M | re.S)
    if not match:
        raise AssertionError(f"missing job {name}")
    return match.group(1)


def run_aggregate(name, needs):
    match = re.search(r"^        run: \|\n((?:^          .*\n|^\s*\n)+)", job(name), re.M)
    if not match:
        raise AssertionError(f"missing aggregate script in {name}")
    return subprocess.run(["bash", "-eo", "pipefail", "-c", textwrap.dedent(match.group(1))],
                          env={**os.environ, "NEEDS_JSON": json.dumps(needs)},
                          capture_output=True, text=True)


def deep_needs(mode="batched", selected="true"):
    return {
        "fast-core": {"result": "success"},
        "select-deep": {"result": "success", "outputs": {
            "mode": "selected", "pointer_mode": mode, "cross_arena": selected,
            "pointer_controls": selected, "fuzz_any": selected,
            "reason_cross_arena": "test", "reason_pointer_controls": "test", "reason_fuzz": "test",
        }},
        "miri-cross-arena": {"result": "success" if selected == "true" else "skipped"},
        "miri-pointer-batches": {"result": "success" if selected == "true" and mode == "batched" else "skipped"},
        "miri-pointer-controls": {"result": "success" if selected == "true" and mode != "batched" else "skipped"},
        "fuzz-corpus-replay": {"result": "success" if selected == "true" else "skipped"},
    }


class AggregateTests(unittest.TestCase):
    def verdict(self, name, needs, success, message):
        result = run_aggregate(name, needs)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertIn(message, result.stdout)

    def test_fast_core_rejects_every_unsuccessful_result(self):
        needs = {"format": {"result": "success"}, "test": {"result": "success"}}
        self.verdict("fast-core", needs, True, "All fast-core jobs succeeded")
        for status in ("failure", "cancelled", "skipped", None):
            with self.subTest(status=status):
                needs["test"]["result"] = status
                self.verdict("fast-core", needs, False, "test:")

    def test_successful_selected_and_unselected_modes(self):
        for mode, selected in (("batched", "true"), ("singleton", "true"), ("batched", "false")):
            with self.subTest(mode=mode, selected=selected):
                self.verdict("deep-safety", deep_needs(mode, selected), True, "All required deep-safety jobs succeeded")

    def test_fast_or_selector_failure_is_not_a_deep_pass(self):
        for name, status in (("fast-core", "cancelled"), ("fast-core", "failure"),
                             ("select-deep", "cancelled"), ("select-deep", "failure")):
            with self.subTest(name=name, status=status):
                needs = deep_needs()
                needs[name]["result"] = status
                self.verdict("deep-safety", needs, False, "NOT EXECUTED" if name == "fast-core" else "required deep lanes are unknown")

    def test_selected_lanes_cannot_fail_cancel_or_disappear(self):
        for lane in ("miri-cross-arena", "miri-pointer-batches", "fuzz-corpus-replay"):
            for status in ("failure", "cancelled", "skipped", None):
                with self.subTest(lane=lane, status=status):
                    needs = deep_needs()
                    if status is None:
                        del needs[lane]
                    else:
                        needs[lane]["result"] = status
                    self.verdict("deep-safety", needs, False, "deep-safety failed")

    def test_batch_failure_stays_failed_after_attribution(self):
        for status in ("success", "failure", "skipped", "cancelled"):
            with self.subTest(status=status):
                needs = deep_needs()
                needs["miri-pointer-batches"]["result"] = "failure"
                needs["miri-pointer-controls"]["result"] = status
                self.verdict("deep-safety", needs, False, "Attribution:")

    def test_missing_selection_is_required_and_singleton_failure_fails(self):
        needs = deep_needs("singleton")
        del needs["select-deep"]["outputs"]["cross_arena"]
        needs["miri-cross-arena"]["result"] = "skipped"
        self.verdict("deep-safety", needs, False, "selected")
        needs = deep_needs("singleton")
        needs["miri-pointer-controls"]["result"] = "cancelled"
        self.verdict("deep-safety", needs, False, "deep-safety failed")

    def test_failed_selector_cannot_publish_partial_outputs(self):
        outputs = re.search(r"^    outputs:\n(.*?)(?=^    steps:)", job("select-deep"), re.M | re.S)
        self.assertIsNotNone(outputs)
        pairs = re.findall(r"^      ([\w_]+): (.*)$", outputs.group(1), re.M)
        self.assertEqual(len(pairs), 9)
        for key, expression in pairs:
            self.assertEqual(expression, "${{ steps.select.outcome == 'success' && steps.select.outputs."
                             + key + " || steps.fallback.outputs." + key + " }}")


if __name__ == "__main__":
    unittest.main()
