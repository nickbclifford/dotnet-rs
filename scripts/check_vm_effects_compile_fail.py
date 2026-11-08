#!/usr/bin/env python3
"""Check that each effects facade compile-fail example fails for borrowing, not API drift."""

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


def main():
    if sys.argv[1:] not in ([], ["multithreading"]):
        raise SystemExit("usage: check_vm_effects_compile_fail.py [multithreading]")
    feature = 'features = ["multithreading"]' if sys.argv[1:] else ""
    text = SOURCE.read_text()
    blocks = re.findall(r"^/// ```compile_fail\n(.*?)^/// ```", text, re.M | re.S)
    if len(blocks) < 6:
        raise SystemExit(f"expected at least six compile-fail examples, found {len(blocks)}")
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
        env = {**os.environ, "CARGO_TARGET_DIR": str(ROOT / "target")}
        for index, block in enumerate(blocks, 1):
            code = "\n".join(line.removeprefix("/// ").removeprefix("///")
                             for line in block.splitlines())
            (project / "src/lib.rs").write_text(code + "\n")
            result = subprocess.run(
                ["cargo", "check", "--manifest-path", str(project / "Cargo.toml"),
                 "--message-format=json"], env=env, text=True, capture_output=True,
            )
            errors = []
            for line in result.stdout.splitlines():
                message = json.loads(line)
                if message.get("reason") == "compiler-message":
                    diagnostic = message["message"]
                    if diagnostic["level"] == "error":
                        errors.append(diagnostic)
            def is_borrow_error(error):
                code = error.get("code")
                if code:
                    return code["code"] in BORROW_CODES
                return bool(re.search(r"lifetime|borrow", error["message"], re.I))
            if result.returncode == 0 or not errors or not all(map(is_borrow_error, errors)):
                print(f"example {index}: expected only lifetime/borrow errors", file=sys.stderr)
                print(result.stdout + result.stderr, file=sys.stderr)
                raise SystemExit(1)
            print(f"example {index}: " + "; ".join(e["message"] for e in errors))
    print(f"validated {len(blocks)} lifetime/borrow failures")


if __name__ == "__main__":
    main()
