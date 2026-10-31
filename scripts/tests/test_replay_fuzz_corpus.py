#!/usr/bin/env python3
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "replay_fuzz_corpus.py"
spec = importlib.util.spec_from_file_location("replay_fuzz_corpus", SCRIPT)
replay = importlib.util.module_from_spec(spec)
spec.loader.exec_module(replay)


def good_log(files=18, runs=19, corpus="corpus/target"):
    return (f"INFO: {files} files found in {corpus}\n"
            f"INFO: seed corpus: files: {files} min: 1b max: 12b\n"
            f"#{runs}\tINITED cov: 1 corp: 17/204b\n"
            f"#{runs}\tDONE cov: 1 corp: 17/204b\n"
            f"Done {runs} runs in 0 second(s)\n")


class LogTests(unittest.TestCase):
    def check(self, log, count=18):
        return replay.verify_log(log, count, "/tmp/work", "corpus/target")

    def test_valid_and_extra_callbacks(self):
        self.assertEqual(self.check(good_log()), 19)
        self.assertEqual(self.check(good_log(129, 182), 129), 182)
        self.assertEqual(self.check("\x1b[32m" + good_log() + "\x1b[0m"), 19)

    def test_fail_closed_logs(self):
        good = good_log()
        cases = {
            "empty": "",
            "missing files": good.replace("INFO: 18 files found in corpus/target\n", ""),
            "missing seed": good.replace("INFO: seed corpus: files: 18 min: 1b max: 12b\n", ""),
            "missing inited": good.replace("#19\tINITED cov: 1 corp: 17/204b\n", ""),
            "skipped": "INFO: A corpus is not provided, starting from an empty corpus\n" + good[good.index("#19"):],
            "truncated": good[:good.index("Done 19")],
            "partial summary": good.replace("0 second(s)", "0 sec"),
            "zero": good_log(18, 0),
            "too few": good_log(18, 18),
            "file mismatch": good.replace("18 files found", "17 files found"),
            "seed mismatch": good.replace("files: 18", "files: 17"),
            "inited mismatch": good.replace("#19\tINITED", "#20\tINITED"),
            "done mismatch": good.replace("#19\tDONE", "#20\tDONE"),
            "final mismatch": good.replace("Done 19 runs", "Done 20 runs"),
            "different path": good.replace("in corpus/target", "in corpus/other"),
            "duplicate": good + good,
            "out of order": good.replace("#19\tINITED cov: 1 corp: 17/204b\n#19\tDONE", "#19\tDONE cov: 1 corp: 17/204b\n#19\tINITED"),
        }
        for name, log in cases.items():
            with self.subTest(name=name), self.assertRaises(replay.ReplayError):
                self.check(log)
        with self.assertRaises(replay.ReplayError):
            self.check(good, 0)


class CorpusTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.corpus = self.root / "work/corpus/target"
        self.corpus.mkdir(parents=True)

    def stage(self):
        subprocess.run(["git", "add", "-A"], cwd=self.root, check=True)

    def inventory(self):
        return replay.inventory(self.root, "work", "corpus/target", "target")

    def test_tracked_manifest_and_rejections(self):
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        seed = self.corpus / "seed"
        seed.write_bytes(b"abc")
        self.stage()
        manifest = self.inventory()
        self.assertEqual(len(manifest), 1)
        self.assertEqual(manifest[0]["bytes"], 3)
        self.assertEqual(manifest[0]["sha256"], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        seed.write_bytes(b"")
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        seed.write_bytes(b"different")
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        seed.unlink()
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        seed.symlink_to("/etc/hosts")
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        seed.unlink()
        seed.write_bytes(b"abc")
        (self.corpus / "unexpected").write_bytes(b"x")
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        (self.corpus / "unexpected").unlink()
        (self.corpus / "linkdir").symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        with self.assertRaises(replay.ReplayError):
            replay.inventory(self.root, "work", "corpus/target", "other")
        with self.assertRaises(replay.ReplayError):
            replay.inventory(self.root, "../work", "corpus/target", "target")

    def test_tracked_symlink(self):
        (self.corpus / "seed").symlink_to("/etc/hosts")
        self.stage()
        with self.assertRaises(replay.ReplayError):
            self.inventory()
        with self.assertRaises(replay.ReplayError):
            replay.inventory(self.root, "work", "corpus/missing", "missing")

    def test_child_exit_and_log_persistence(self):
        (self.corpus / "seed").write_bytes(b"abc")
        self.stage()
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        cargo = bin_dir / "cargo"
        cargo.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ARGV_LOG\"\ncat \"$FAKE_LOG\"\nexit \"$FAKE_STATUS\"\n")
        cargo.chmod(0o755)
        fake_log = self.root / "fake.log"
        fake_log.write_text(good_log(1, 2))
        output = self.root / "output"
        shim = ("import importlib.util,sys; "
                "s=importlib.util.spec_from_file_location('r',sys.argv[1]); "
                "m=importlib.util.module_from_spec(s); s.loader.exec_module(m); "
                "sys.exit(m.replay(sys.argv[2], 'work', 'corpus/target', 'target', sys.argv[3]))")
        env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", FAKE_LOG=str(fake_log),
                   ARGV_LOG=str(self.root / "argv"))
        for status, expected in ((0, 0), (7, 1)):
            env["FAKE_STATUS"] = str(status)
            result = subprocess.run([sys.executable, "-c", shim, str(SCRIPT), str(self.root), str(output)],
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, expected, result.stderr)
            self.assertIn("Done 2 runs", (output / "target.log").read_text())
            self.assertTrue((output / "target.manifest.json").exists())
        self.assertIn("replay child exited 7", (output / "target.log").read_text())
        self.assertEqual((self.root / "argv").read_text().splitlines(),
                         ["+nightly-2026-05-27", "fuzz", "run", "target", "corpus/target", "--", "-runs=0"])
        fake_log.write_text(good_log(1, 2).replace("Done 2 runs in 0 second(s)\n", ""))
        env["FAKE_STATUS"] = "0"
        result = subprocess.run([sys.executable, "-c", shim, str(SCRIPT), str(self.root), str(output)],
                                env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("Replay validation failed", (output / "target.log").read_text())


if __name__ == "__main__":
    unittest.main()
