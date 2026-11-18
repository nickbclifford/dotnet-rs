import pathlib
import subprocess
import unittest


class VmOnlyGuardTest(unittest.TestCase):
    def test_vm_only_source_is_rejected_before_runtime_execution(self):
        root = pathlib.Path(__file__).resolve().parents[2]
        fixture = root / "crates/dotnet-cli/tests/debug_fixtures/finalizer_forced_monitor_deadlock.cs"
        result = subprocess.run(
            ["bash", str(root / "scripts/diff_run.sh"), str(fixture)],
            capture_output=True, text=True, timeout=5,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("VM-only fixture cannot be run on dotnet", result.stderr)


if __name__ == "__main__":
    unittest.main()
