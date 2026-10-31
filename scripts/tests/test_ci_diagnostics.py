#!/usr/bin/env python3
"""Synthetic end-to-end checks of phase evidence, signals and exit preservation."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "ci_diagnostics.py"


class DiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name) / "diagnostics"
        # Ignore any wrapper defaults exported by the CI job running these tests.
        inherited = {key: value for key, value in os.environ.items() if not key.startswith("CI_DIAG_")}
        self.env = {**inherited, "GITHUB_SHA": "a" * 40, "MIRIFLAGS": "-Zmiri-tree-borrows test"}

    def base(self, phase="execute", case="case"):
        return [sys.executable, str(SCRIPT), "run", "--dir", str(self.directory),
                "--phase", phase, "--case", case, "--timeout", "5",
                "--subphase", "sysroot", "--features", "no-features", "--toolchain", "nightly-test",
                "--cache", "cold"]

    def launch(self, code, phase="execute", options=(), timeout="5"):
        cmd = self.base(phase)
        cmd[cmd.index("--timeout") + 1] = timeout
        return subprocess.run(cmd + list(options) + ["--", sys.executable, "-u", "-c", code],
                              env=self.env, capture_output=True, timeout=10)

    def evidence(self, phase="execute"):
        prefix = self.directory / f"{phase}--case"
        return tuple(json.loads(Path(str(prefix) + suffix).read_text()) for suffix in (".start.json", ".status.json"))

    def log(self, phase="execute"):
        return (self.directory / f"{phase}--case.log").read_text()

    def test_success_streams_metadata_and_required_summary(self):
        result = self.launch("import sys; print('out'); print('err', file=sys.stderr); print('1 passed; 0 ignored')",
                             options=("--require", "1 passed; 0 ignored"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b"out", result.stdout)
        self.assertIn(b"err", result.stdout)
        self.assertIn("out\n", self.log())
        self.assertIn("err\n", self.log())
        start, status = self.evidence()
        self.assertEqual(start["sha"], "a" * 40)
        self.assertEqual((start["features"], start["toolchain"], start["cache"]), ("no-features", "nightly-test", "cold"))
        self.assertEqual(start["miriflags"], "-Zmiri-tree-borrows test")
        self.assertEqual(start["command"][-1].split(";")[0], "import sys")
        self.assertTrue(start["started_utc"].endswith("Z"))
        self.assertTrue(status["ended_utc"].endswith("Z"))
        self.assertEqual((status["classification"], status["child_returncode"], status["exit_code"]), ("success", 0, 0))

    def test_failure_classes_and_original_status(self):
        cases = [
            ("setup", "error: could not install toolchain", 42, "setup-error"),
            ("compile", "error: could not compile crate", 101, "setup-error"),
            ("compile", "thread 'rustc' panicked at compiler", 101, "setup-error"),
            ("execute", "error: Undefined Behavior: fake miri diagnostic", 91, "miri-ub"),
            ("execute", "test example ... FAILED", 101, "test-failure"),
            ("execute", "thread 'x' panicked at line 1", 7, "test-failure"),
            ("execute", "Replay validation failed: expected exactly one DONE, found 0", 1, "validation-guard-failure"),
            ("guard", "manifest mismatch", 3, "validation-guard-failure"),
            ("execute", "unexpected error", 73, "unclassified-process-error"),
            ("execute", "Miri found no undefined behavior", 1, "unclassified-process-error"),
            ("execute", "Replay validation failed: command not found", 1, "unclassified-process-error"),
            ("execute", "Replay validation failed: replay child exited 124", 124, "unclassified-process-error"),
            ("execute", "expected exactly 1 passing controls", 1, "validation-guard-failure"),
            ("execute", "killed without deadline", 137, "unclassified-process-error"),
        ]
        for index, (phase, message, rc, expected) in enumerate(cases):
            with self.subTest(phase=phase, message=message):
                self.directory = Path(self.temp.name) / str(index)
                result = self.launch(f"import sys; print({message!r}, file=sys.stderr); sys.exit({rc})", phase=phase)
                self.assertEqual(result.returncode, rc, result.stderr)
                start, status = self.evidence(phase)
                self.assertEqual(status["classification"], expected)
                self.assertEqual(status["child_returncode"], rc)
                self.assertFalse(status["deadline_expired"])
                self.assertEqual(start["subphase"], "sysroot")
                self.assertIn(message, self.log(phase))

    def test_child_signal_is_not_assumed_to_be_timeout(self):
        result = self.launch("import os, signal; os.kill(os.getpid(), signal.SIGKILL)", phase="setup")
        self.assertEqual(result.returncode, 137)
        _, status = self.evidence("setup")
        self.assertEqual(status["child_returncode"], -signal.SIGKILL)
        self.assertEqual(status["classification"], "cancelled-or-unknown")
        self.assertFalse(status["deadline_expired"])

    def test_missing_summary_is_nonzero_and_not_test_success(self):
        result = self.launch("print('running 0 tests')", options=("--require", "test result: ok. 1 passed; 0 failed; 0 ignored;"))
        self.assertEqual(result.returncode, 1)
        _, status = self.evidence()
        self.assertEqual(status["classification"], "validation-guard-failure")
        self.assertEqual(status["child_returncode"], 0)
        self.assertEqual(len(status["missing_required_output"]), 1)

    def test_timeout_is_evidenced_and_kills_child_group(self):
        code = ("import os, signal, subprocess, sys, time; "
                "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']); "
                "print('before deadline', flush=True); time.sleep(60)")
        result = self.launch(code, timeout="0.2")
        self.assertEqual(result.returncode, 124, result.stderr)
        _, status = self.evidence()
        self.assertEqual(status["classification"], "budget-exhaustion")
        self.assertTrue(status["deadline_expired"])
        self.assertIsNone(status["received_signal"])
        self.assertIn("before deadline", self.log())

    @unittest.skipUnless(Path("/proc").is_dir(), "descendant groups are discovered through /proc")
    def test_timeout_kills_grandchild_in_its_own_process_group(self):
        # Mimics GNU timeout inside the pointer script: a new group that also holds the output pipe.
        pid_file = Path(self.temp.name) / "grandchild.pid"
        code = ("import subprocess, sys, time; "
                "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], process_group=0); "
                f"open({str(pid_file)!r}, 'w').write(str(child.pid)); "
                "print('spawned', flush=True); time.sleep(60)")
        began = time.monotonic()
        result = self.launch(code, timeout="1")
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertLess(time.monotonic() - began, 9)
        grandchild = int(pid_file.read_text())
        limit = time.monotonic() + 3
        while Path(f"/proc/{grandchild}").exists() and time.monotonic() < limit:
            stat = Path(f"/proc/{grandchild}/stat").read_text() if Path(f"/proc/{grandchild}").exists() else ""
            if stat.rsplit(")", 1)[-1].split()[:1] == ["Z"]:
                break  # killed, awaiting reaping by init
            time.sleep(0.05)
        alive = Path(f"/proc/{grandchild}").exists() and Path(f"/proc/{grandchild}/stat").read_text().rsplit(")", 1)[-1].split()[0] != "Z"
        if alive:
            os.kill(grandchild, signal.SIGKILL)
        self.assertFalse(alive, "grandchild in a separate process group survived the deadline")
        self.assertEqual(self.evidence()[1]["classification"], "budget-exhaustion")

    def test_term_and_int_save_partial_status_and_nonzero(self):
        for sig in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(signal=sig):
                self.directory = Path(self.temp.name) / str(sig)
                process = subprocess.Popen(self.base() + ["--", sys.executable, "-u", "-c",
                                           "import time; print('started', flush=True); time.sleep(60)"],
                                           env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                try:
                    log = self.directory / "execute--case.log"
                    limit = time.monotonic() + 5
                    while (not log.exists() or 'started' not in log.read_text()) and time.monotonic() < limit:
                        time.sleep(0.01)
                    self.assertIn("started", log.read_text())
                    process.send_signal(sig)
                    stdout, stderr = process.communicate(timeout=6)
                    self.assertEqual(process.returncode, 128 + sig, stderr)
                    self.assertIn(b"started", stdout)
                    _, status = self.evidence()
                    self.assertEqual(status["classification"], "cancelled-or-unknown")
                    self.assertEqual(status["received_signal"], sig)
                    self.assertFalse(status["deadline_expired"])
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.communicate()

    def test_finalize_missing_status_without_inventing_success(self):
        result = self.launch("print('done')")
        self.assertEqual(result.returncode, 0)
        start, original = self.evidence()
        status_path = self.directory / "execute--case.status.json"
        cmd = [sys.executable, str(SCRIPT), "finalize", "--dir", str(self.directory),
               "--phase", "execute", "--case", "case", "--outcome"]
        self.assertEqual(subprocess.run(cmd + ["cancelled"], env=self.env, capture_output=True).returncode, 0)
        self.assertEqual(json.loads(status_path.read_text()), original)  # no overwrite
        for outcome, expected in (("cancelled", "cancelled-or-unknown"), ("success", "missing-status"),
                                  ("failure", "missing-status")):
            status_path.unlink()
            self.assertEqual(subprocess.run(cmd + [outcome], env=self.env, capture_output=True).returncode, 0)
            status = json.loads(status_path.read_text())
            self.assertEqual(status["classification"], expected)
            self.assertTrue(status["finalized_from_start_marker"])
            self.assertIsNone(status["child_returncode"])
            self.assertEqual(status["subphase"], start["subphase"])
        status_path.unlink()
        (self.directory / "execute--case.start.json").unlink()
        self.assertEqual(subprocess.run(cmd + ["cancelled"], env=self.env, capture_output=True).returncode, 0)
        self.assertFalse(status_path.exists())

    def test_path_safety_collision_invalid_timeout_and_spawn_failure(self):
        for name in ("../outside", "/absolute", "..", "bad/name", "bad\nname"):
            result = subprocess.run(self.base(case=name) + ["--", "true"], env=self.env, capture_output=True)
            self.assertNotEqual(result.returncode, 0, name)
        for timeout in ("0", "-1", "nan", "inf"):
            cmd = self.base()
            cmd[cmd.index("--timeout") + 1] = timeout
            self.assertNotEqual(subprocess.run(cmd + ["--", "true"], env=self.env, capture_output=True).returncode, 0)
        self.assertEqual(self.launch("print('first')").returncode, 0)
        self.assertNotEqual(self.launch("print('second')").returncode, 0)
        self.assertIn("first", self.log())
        self.assertNotIn("second", self.log())
        self.directory = Path(self.temp.name) / "spawn"
        result = subprocess.run(self.base("setup") + ["--", "/does-not-exist/command"], env=self.env, capture_output=True)
        self.assertEqual(result.returncode, 127)
        _, status = self.evidence("setup")
        self.assertEqual(status["classification"], "setup-error")
        self.assertIn("could not start command", self.log("setup"))

    def test_job_deadline_caps_timeout_and_skips_spent_budget(self):
        self.env["CI_DIAG_DEADLINE"] = str(time.time() + 0.3)
        began = time.monotonic()
        result = self.launch("import time; print('started', flush=True); time.sleep(60)", timeout="30")
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertLess(time.monotonic() - began, 8)
        start, status = self.evidence()
        self.assertEqual(status["classification"], "budget-exhaustion")
        self.assertLessEqual(start["effective_timeout_seconds"], 0.3)
        self.assertEqual(start["timeout_seconds"], 30)
        self.directory = Path(self.temp.name) / "spent"
        self.env["CI_DIAG_DEADLINE"] = str(time.time() - 1)
        marker = Path(self.temp.name) / "ran"
        result = self.launch(f"open({str(marker)!r}, 'w').close()")
        self.assertEqual(result.returncode, 124, result.stderr)
        _, status = self.evidence()
        self.assertEqual((status["classification"], status["deadline_expired"]), ("budget-exhaustion", True))
        self.assertIn("command not started", self.log())
        self.assertFalse(marker.exists())

    def test_environment_defaults_supply_directory_and_identity(self):
        env = {**self.env, "CI_DIAG_DIR": str(self.directory), "CI_DIAG_FEATURES": "multithreading",
               "CI_DIAG_TOOLCHAIN": "nightly-env", "CI_DIAG_CACHE": "rust-cache-hit=true"}
        cmd = [sys.executable, str(SCRIPT), "run", "--phase", "setup", "--case", "case", "--timeout", "5",
               "--", sys.executable, "-c", "print('ok')"]
        self.assertEqual(subprocess.run(cmd, env=env, capture_output=True).returncode, 0)
        start, _ = self.evidence("setup")
        self.assertEqual((start["features"], start["toolchain"], start["cache"]),
                         ("multithreading", "nightly-env", "rust-cache-hit=true"))

    def summarize(self, *options, outcome="failure"):
        return subprocess.run([sys.executable, str(SCRIPT), "summarize", "--dir", str(self.directory),
                               "--outcome", outcome, "--title", "Miri | demo", *options],
                              capture_output=True, text=True)

    def test_summarize_finalizes_interrupted_phases_and_never_fails(self):
        result = self.summarize()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("No phase evidence was recorded", result.stdout)
        self.assertEqual(self.launch("print('compiled')", phase="compile").returncode, 0)
        self.assertEqual(self.launch("print('ok')").returncode, 0)
        # An interrupted guard phase leaves only a start marker.
        self.directory.joinpath("guard--case.start.json").write_text(json.dumps(
            {"phase": "guard", "case": "case", "subphase": "", "started_utc": "9999-01-01T00:00:00.000Z"}))
        result = self.summarize("--advisory", outcome="cancelled")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("### CI diagnostics: Miri | demo", result.stdout)
        self.assertIn("**Advisory diagnostic failure:** 1 phase(s)", result.stdout)
        self.assertIn("| guard | - | case | cancelled-or-unknown |", result.stdout)
        self.assertIn("| compile | sysroot | case | success |", result.stdout)
        self.assertLess(result.stdout.index("| compile"), result.stdout.index("| execute"))
        self.assertEqual(json.loads((self.directory / "guard--case.status.json").read_text())["classification"],
                         "cancelled-or-unknown")
        self.assertEqual((self.directory / "summary.md").read_text(), result.stdout)
        blocking = self.summarize(outcome="failure")
        self.assertIn("**Blocking failure:** 1 phase(s)", blocking.stdout)
        (self.directory / "guard--case.start.json").write_text("{not json")
        (self.directory / "guard--case.status.json").unlink()
        result = self.summarize(outcome="success")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("All 2 recorded phase(s) succeeded.", result.stdout)
        self.assertIn("Unreadable evidence: guard--case.start.json", result.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
