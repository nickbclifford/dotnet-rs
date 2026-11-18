"""Exercise the real invocation guard without cleaning or building the workspace."""
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "check_build_script_regressions.sh"


class BuildInvocationGuardTests(unittest.TestCase):
    def run_guard(self, command):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "build.log"
            result = subprocess.run(
                ["bash", "-c", 'source "$1"; assert_no_dotnet_invocations "$2" bash -c "$3"',
                 "guard", str(SCRIPT), str(log), command],
                capture_output=True, text=True, timeout=10,
            )
            return result, log.read_text()

    def test_quiet_invocation_fails_even_if_caller_ignores_status(self):
        result, _ = self.run_guard("dotnet build --nologo >/dev/null 2>&1 || true")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unexpected dotnet invocation", result.stderr)

    def test_project_names_in_output_are_not_invocations(self):
        result, log = self.run_guard("printf '%s\\n' 'checking example.csproj with msbuild conventions'")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("example.csproj", log)

    def test_unrelated_build_failure_is_preserved(self):
        result, _ = self.run_guard("exit 17")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("guarded build exited 17", result.stderr)


if __name__ == "__main__":
    unittest.main()
