#!/usr/bin/env python3
"""Synthetic guard and command-expansion tests; no Miri suite is executed."""
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("miri_advisory", ROOT / "scripts/miri_advisory.py")
miri = importlib.util.module_from_spec(spec)
spec.loader.exec_module(miri)


def listing(names, ancillary=1):
    return "".join(f"{name}: test\n" for name in names) + f"\n{len(names)} tests, 0 benchmarks\n" + "0 tests, 0 benchmarks\n" * ancillary


class AdvisoryTests(unittest.TestCase):
    def setUp(self):
        self.data = miri.manifest()

    def test_review_manifest_and_matrix_exactly_once(self):
        review = (ROOT / "REVIEW.md").read_text().split("### Advisory exact singleton manifests", 1)[1]
        expected = dict(re.findall(r"^\| ([AV]\d\d) \| `([^`]+)` \| 1 \|", review, re.MULTILINE))
        self.assertEqual(self.data, expected)
        rows = miri.rows(self.data)
        self.assertEqual(len(rows), 54)
        for config, count in [("default-features", 38), ("no-features", 8), ("multithreading", 8)]:
            shards = [r["shard"] for r in rows if r["configuration"] == config]
            self.assertEqual(len(shards), count)
            self.assertEqual(len(set(shards)), count)
            self.assertTrue(all(r["crate"] == miri.CONFIGS[config][0] for r in rows if r["configuration"] == config))

    def test_manifest_rejects_missing_extra_duplicate_and_invalid(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "m.json"
            variants = []
            missing = dict(self.data); missing.pop("A01"); variants.append(json.dumps(missing))
            extra = dict(self.data); extra["A39"] = "extra"; variants.append(json.dumps(extra))
            duplicate = dict(self.data); duplicate["A02"] = duplicate["A01"]; variants.append(json.dumps(duplicate))
            invalid = dict(self.data); invalid["V01"] = "--ignored"; variants.append(json.dumps(invalid))
            variants.append('{"A01":"first", "A01":"second"}')
            for value in variants:
                with self.subTest(value=value[:80]):
                    path.write_text(value)
                    with self.assertRaises(ValueError):
                        miri.manifest(path)

    def test_listing_union_and_empty_ancillary(self):
        for config, (_, _, prefix, _, _) in miri.CONFIGS.items():
            with self.subTest(config=config):
                names = [name for key, name in self.data.items() if key.startswith(prefix)]
                ancillary = 1 if prefix == "A" else 2
                good = listing(names, ancillary)
                empty = listing([], ancillary)
                miri.verify_lists(config, self.data, good, empty)
                bads = [listing(names[:-1], ancillary),
                        listing(names[:-1] + ["unreviewed::test"], ancillary),
                        listing(names[:-1] + [names[0]], ancillary),
                        listing(names, 0),
                        listing(names, ancillary - 1) + listing(["new::test"], 0),
                        good.replace("0 benchmarks", "1 benchmark", 1),
                        good + "truncated: test\n"]
                for bad in bads:
                    with self.subTest(bad=bad[-100:]), self.assertRaises(ValueError):
                        miri.verify_lists(config, self.data, bad, empty)
                with self.assertRaises(ValueError):
                    miri.verify_lists(config, self.data, good, listing([names[0]], ancillary))

    def test_exact_result_with_diagnostics_and_failure_modes(self):
        name = self.data["A01"]
        good = f"running 1 test\ntest {name} ... warning: integer-to-pointer cast\nmore warning text\nok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 37 filtered out\n"
        miri.verify_result(good, name)
        for bad in [good.replace("running 1 test", "running 0 tests"),
                    good.replace(name, "wrong::test"), good.replace("1 passed", "0 passed"),
                    good.replace("0 ignored", "1 ignored"), good.replace("0 failed", "1 failed"),
                    good.replace("test result: ok.", "test result: FAILED."),
                    good.replace("test result:", "truncated result:"), good + good]:
            with self.subTest(bad=bad[-90:]), self.assertRaisesRegex(ValueError, "zero-ignored-or-unverified-count"):
                miri.verify_result(bad, name)

    def test_fake_cargo_original_filters_and_exact_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "cargo"
            fake.write_text('''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
with open(os.environ["CALLS"], "a") as f: f.write(json.dumps(sys.argv[1:]) + "\\n")
if os.environ.get("FAIL"): sys.exit(int(os.environ["FAIL"]))
if os.environ.get("UB"):
    print("error: Undefined Behavior: simulated", file=sys.stderr)
    sys.exit(1)
if "--list" in sys.argv:
    print(Path(os.environ["IGNORED" if "--ignored" in sys.argv else "LIST"]).read_text(), end="")
else:
    name = sys.argv[sys.argv.index("--exact")+1]
    print("running 1 test\\ntest " + name + " ... ok\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out")
''')
            fake.chmod(0o755)
            env = dict(os.environ, PATH=tmp + os.pathsep + os.environ["PATH"], CALLS=tmp + "/calls",
                       LIST=tmp + "/list", IGNORED=tmp + "/ignored")
            for config, (crate, features, prefix, _, filters) in miri.CONFIGS.items():
                names = [name for key, name in self.data.items() if key.startswith(prefix)]
                anc = 1 if prefix == "A" else 2
                Path(env["LIST"]).write_text(listing(names, anc)); Path(env["IGNORED"]).write_text(listing([], anc))
                base = [sys.executable, str(ROOT / "scripts/miri_advisory.py")]
                subprocess.run(base + ["verify", "--configuration", config], env=env, check=True, capture_output=True)
                shard = prefix + "01"
                subprocess.run(base + ["execute", "--configuration", config, "--shard", shard], env=env, check=True, capture_output=True)
                calls = [json.loads(line) for line in Path(env["CALLS"]).read_text().splitlines()]
                Path(env["CALLS"]).unlink()
                cargo = [miri.TOOLCHAIN, "miri", "test", "-p", crate, *features]
                self.assertEqual(calls, [cargo + ["--", "--list", *filters],
                                         cargo + ["--", "--list", "--ignored", *filters],
                                         cargo + ["--lib", "--", "--exact", self.data[shard], "--test-threads=1"]])
                failed = subprocess.run(base + ["execute", "--configuration", config, "--shard", shard],
                                        env=dict(env, FAIL="17"), capture_output=True)
                self.assertEqual(failed.returncode, 17)
                Path(env["CALLS"]).unlink()
                failed = subprocess.run(base + ["execute", "--configuration", config, "--shard", "V01" if prefix == "A" else "A01"], env=env, capture_output=True)
                self.assertNotEqual(failed.returncode, 0)

            # Diagnostics wrapper must retain Miri UB rather than treating the
            # advisory child failure as success or a count-guard failure.
            diag = [sys.executable, str(ROOT / "scripts/ci_diagnostics.py"), "run", "--dir", tmp + "/diag",
                    "--phase", "execute", "--case", "ub", "--timeout", "10", "--"]
            child = [sys.executable, str(ROOT / "scripts/miri_advisory.py"), "execute",
                     "--configuration", "default-features", "--shard", "A01"]
            result = subprocess.run(diag + child, env=dict(env, UB="1"), capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            status = json.loads(Path(tmp + "/diag/execute--ub.status.json").read_text())
            self.assertEqual(status["classification"], "miri-ub")
            self.assertIn("error: Undefined Behavior", Path(tmp + "/diag/execute--ub.log").read_text())


if __name__ == "__main__":
    unittest.main()
