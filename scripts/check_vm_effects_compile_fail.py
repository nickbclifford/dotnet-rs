#!/usr/bin/env python3
"""Check that each effects facade compile-fail example fails for its expected reason."""

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/dotnet-vm-ops/src/effects.rs"
BORROW_CODES = {"E0373", "E0499", "E0502", "E0505", "E0521", "E0597", "E0716"}
COMPILE_FAIL_FENCE = re.compile(
    r"^/// ```compile_fail(?P<annotations>(?:,[A-Za-z0-9_-]+)*)\r?\n"
    r"(?P<code>.*?)^/// ```[ \t]*$",
    re.M | re.S,
)


def compile_project(project, code, env):
    (project / "src/lib.rs").write_text(code + "\n")
    return subprocess.run(
        ["cargo", "check", "--manifest-path", str(project / "Cargo.toml"),
         "--message-format=json"],
        env=env,
        text=True,
        capture_output=True,
    )


def compiler_errors(stdout):
    errors = []
    for line in stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-message":
            diagnostic = message["message"]
            if diagnostic["level"] == "error":
                errors.append(diagnostic)
    return errors


def main():
    if sys.argv[1:] not in ([], ["multithreading"]):
        raise SystemExit("usage: check_vm_effects_compile_fail.py [multithreading]")
    feature = 'features = ["multithreading"]' if sys.argv[1:] else ""
    text = SOURCE.read_text()
    blocks = list(COMPILE_FAIL_FENCE.finditer(text))
    if not blocks:
        raise SystemExit("found no compile-fail examples")
    with tempfile.TemporaryDirectory(prefix="vm-effects-doctest-") as temp:
        project = Path(temp)
        (project / "src").mkdir()
        shutil.copyfile(ROOT / "Cargo.lock", project / "Cargo.lock")
        (project / "Cargo.toml").write_text(f'''[package]
name = "vm-effects-doctest-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
dotnet-vm-ops = {{ path = "{ROOT / 'crates/dotnet-vm-ops'}", default-features = false, {feature} }}
dotnet-value = {{ path = "{ROOT / 'crates/dotnet-value'}", default-features = false, {feature} }}
''')
        env = os.environ.copy()
        env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))

        positive_control = """\
use dotnet_vm_ops::effects::{NativeFuture, VmEffects};

fn use_effects(_: Option<VmEffects>) {}
fn use_future(_: Option<NativeFuture>) {}
"""
        result = compile_project(project, positive_control, env)
        if result.returncode != 0:
            print("positive control: expected the facade types to compile", file=sys.stderr)
            print(result.stdout + result.stderr, file=sys.stderr)
            raise SystemExit(1)
        print("positive control: facade types compile")

        for index, block in enumerate(blocks, 1):
            code = "\n".join(
                line.removeprefix("/// ").removeprefix("///")
                for line in block["code"].splitlines()
            )
            result = compile_project(project, code, env)
            errors = compiler_errors(result.stdout)
            annotated_codes = {
                annotation
                for annotation in block["annotations"].split(",")
                if re.fullmatch(r"E\d{4}", annotation)
            }

            def is_borrow_error(error):
                code = error.get("code")
                if code:
                    return code["code"] in BORROW_CODES
                return bool(re.search(r"lifetime|borrow", error["message"], re.I))

            if annotated_codes:
                actual_codes = {
                    error.get("code", {}).get("code") for error in errors
                }
                expected_errors = (
                    result.returncode != 0
                    and bool(errors)
                    and actual_codes == annotated_codes
                    and all(error.get("code") for error in errors)
                )
                expectation = "compiler error(s) " + ", ".join(sorted(annotated_codes))
            else:
                expected_errors = (
                    result.returncode != 0
                    and bool(errors)
                    and all(map(is_borrow_error, errors))
                )
                expectation = "only lifetime/borrow errors"

            if not expected_errors:
                print(f"example {index}: expected {expectation}", file=sys.stderr)
                print(result.stdout + result.stderr, file=sys.stderr)
                raise SystemExit(1)
            print(f"example {index}: " + "; ".join(e["message"] for e in errors))
    print(f"validated {len(blocks)} compile-fail examples")


if __name__ == "__main__":
    main()
