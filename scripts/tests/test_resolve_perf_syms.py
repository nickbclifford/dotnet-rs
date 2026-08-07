#!/usr/bin/env python3
"""Regression tests for Firefox-Profiler perf-script post-processing."""

import subprocess
import sys
import unittest
from pathlib import Path


RESOLVER = Path(__file__).resolve().parents[1] / "resolve_perf_syms.py"


class ResolvePerfSymbolsTests(unittest.TestCase):
    def run_resolver(self, script):
        return subprocess.run(
            [sys.executable, str(RESOLVER), "--target-comm", "end_to_end"],
            input=script,
            text=True,
            capture_output=True,
            check=True,
        )

    def test_reports_threads_shallow_stacks_and_foreign_processes(self):
        result = self.run_resolver(
            "end_to_end  100/100  1.000: 100 cycles:\n"
            "\t7f00 root (/tmp/end_to_end)\n"
            "\t7e00 leaf (/tmp/end_to_end)\n"
            "\n"
            "worker  100/101  2.000: 50 cycles:\n"
            "\t7d00 worker_leaf (/tmp/worker)\n"
            "\n"
            "end_to_end  100/100  3.000: 25 cycles:\n"
            "\n"
        )

        self.assertIn("samples=3 empty_stacks=1 (33% of count) single_frame=1 (33% of count)", result.stderr)
        self.assertIn(
            "unresolved-leaf=14.3%  foreign-process=28.6%  single-frame=28.6%",
            result.stderr,
        )
        self.assertIn("processes by cycles: end_to_end=71%, worker=29%", result.stderr)
        self.assertIn("threads by cycles: end_to_end/100=71%, worker/101=29%", result.stderr)
        self.assertIn("29% of cycles are in OTHER processes", result.stderr)
        self.assertIn("only 3 samples", result.stderr)

    def test_keeps_the_perf_script_shape_when_symbols_stay_unresolved(self):
        result = self.run_resolver(
            "end_to_end  100/100  1.000: 10 cycles:\n"
            "\t7f00 [unknown] (/tmp/libc.so+0x20)\n"
            "\n"
        )

        self.assertIn("\t7f00 [unknown] (/tmp/libc.so)\n", result.stdout)
        self.assertIn("unresolved-leaf=100.0%", result.stderr)


if __name__ == "__main__":
    unittest.main()
