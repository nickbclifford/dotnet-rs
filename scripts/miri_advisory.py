#!/usr/bin/env python3
"""Miri-aware singleton partition and execution guard for advisory suites.

The manifest drives the matrix, never the discovery command: discovery always uses
exactly the old full-crate/filter selection, including ancillary binaries and docs.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys

MANIFEST = Path(__file__).with_name("miri_advisory_manifest.json")
TOOLCHAIN = "+nightly-2026-05-27"
CONFIGS = {
    "default-features": ("dotnet-assemblies", [], "A", 38, []),
    "no-features": ("dotnet-vm", ["--no-default-features"], "V", 8, ["jmp_tests", "tail_calls", "fault_tests"]),
    "multithreading": ("dotnet-vm", ["--no-default-features", "--features", "multithreading"], "V", 8, ["jmp_tests", "tail_calls", "fault_tests"]),
}
NAME = re.compile(r"[A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*\Z")
SUMMARY = re.compile(r"(\d+) tests?, (\d+) benchmarks?\Z")


def manifest(path=MANIFEST):
    # Reject duplicate JSON keys instead of silently accepting the last occurrence.
    def pairs(items):
        data = {}
        for key, value in items:
            if key in data:
                raise ValueError(f"duplicate shard {key}")
            data[key] = value
        return data
    data = json.loads(Path(path).read_text(), object_pairs_hook=pairs)
    expected = {f"{prefix}{i:02d}" for prefix, n in (("A", 38), ("V", 8)) for i in range(1, n + 1)}
    if not isinstance(data, dict) or set(data) != expected:
        raise ValueError("manifest must contain exactly A01..A38 and V01..V08")
    if any(not isinstance(name, str) or not NAME.fullmatch(name) for name in data.values()):
        raise ValueError("invalid test selector")
    for prefix in ("A", "V"):
        names = [name for key, name in data.items() if key.startswith(prefix)]
        if len(names) != len(set(names)):
            raise ValueError(f"duplicate {prefix} selector")
    return data


def rows(data):
    return [{"crate": crate, "configuration": config, "shard": key}
            for config, (crate, _, prefix, _, _) in CONFIGS.items()
            for key in sorted(data) if key.startswith(prefix)]


def parse_list(text):
    """Parse libtest listing sections; summaries delimit each binary/doc target."""
    sections, pending = [], []
    for line in text.splitlines():
        if not line.strip():
            continue
        summary = SUMMARY.fullmatch(line)
        if summary:
            count, benches = map(int, summary.groups())
            if benches or count != len(pending):
                raise ValueError(f"listing count/benchmark mismatch: {line}")
            sections.append(pending)
            pending = []
        elif line.endswith(": test") and NAME.fullmatch(line[:-6]):
            pending.append(line[:-6])
        else:
            raise ValueError(f"unexpected listing line: {line!r}")
    if pending or not sections:
        raise ValueError("incomplete or empty listing")
    return sections


def verify_lists(config, data, listed, ignored):
    _, _, prefix, count, _ = CONFIGS[config]
    actual = parse_list(listed)
    ignored_sections = parse_list(ignored)
    # Original cargo command includes lib and doctests, and VM's integration binary.
    minimum_sections = 2 if prefix == "A" else 3
    if len(actual) < minimum_sections or len(ignored_sections) != len(actual):
        raise ValueError("missing original ancillary binary/doctest listing")
    if any(actual[1:]) or any(ignored_sections):
        raise ValueError("original ancillary selection gained tests, or selection contains ignored tests")
    expected = [name for shard, name in data.items() if shard.startswith(prefix)]
    if len(actual[0]) != count or Counter(actual[0]) != Counter(expected):
        raise ValueError(f"original Miri listing differs from {count}-selector shard union; "
                         f"missing={list((Counter(expected)-Counter(actual[0])).elements())}, "
                         f"unexpected={list((Counter(actual[0])-Counter(expected)).elements())}")
    print(f"{config}: {count} non-ignored library tests; {len(actual)-1} empty ancillary listings; "
          f"{count} singleton shards partition the original Miri selection exactly once", flush=True)


def cargo_base(config):
    crate, args, _, _, _ = CONFIGS[config]
    return ["cargo", TOOLCHAIN, "miri", "test", "-p", crate, *args]


def listing(config, ignored):
    filters = CONFIGS[config][4]
    cmd = [*cargo_base(config), "--", "--list", *(["--ignored"] if ignored else []), *filters]
    print("+ " + shlex.join(cmd), flush=True)
    result = subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    print(result.stderr, file=sys.stderr, end="", flush=True)
    print(result.stdout, end="", flush=True)
    if result.returncode:
        raise subprocess.CalledProcessError(result.returncode, cmd)
    return result.stdout


def verify_result(text, selector):
    running = re.findall(r"^running (\d+) tests?$", text, re.MULTILINE)
    result = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", text, re.MULTILINE)
    # Miri diagnostics can appear between libtest's `...` and its eventual `ok`.
    started = re.findall(r"^test ([A-Za-z0-9_:]+) \.\.\.", text, re.MULTILINE)
    if running != ["1"] or result != [("1", "0", "0")] or started != [selector]:
        raise ValueError("zero-ignored-or-unverified-count: expected exactly the named test, 1 passed, 0 failed, 0 ignored")


def execute(config, shard, data):
    if shard not in data or not shard.startswith(CONFIGS[config][2]):
        raise ValueError("shard is not in this configuration")
    selector = data[shard]
    cmd = [*cargo_base(config), "--lib", "--", "--exact", selector, "--test-threads=1"]
    print("+ " + shlex.join(cmd), flush=True)
    # Stream as it happens for timeout/UB artifacts; preserve both streams and child status.
    with subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1) as process:
        lines = []
        for line in process.stdout:
            print(line, end="", flush=True)
            lines.append(line)
        rc = process.wait()
    if rc:
        return rc if rc > 0 else 128 - rc
    verify_result("".join(lines), selector)
    print(f"Verified {shard}: {selector} passed exactly once, ignored 0", flush=True)
    return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("matrix", "verify", "execute"))
    parser.add_argument("--configuration", choices=CONFIGS)
    parser.add_argument("--shard")
    args = parser.parse_args()
    try:
        data = manifest()
        if args.action == "matrix":
            print(json.dumps({"include": rows(data)}, separators=(",", ":")))
        else:
            if not args.configuration:
                raise ValueError("configuration required")
            if args.action == "verify":
                verify_lists(args.configuration, data, listing(args.configuration, False), listing(args.configuration, True))
            else:
                return execute(args.configuration, args.shard, data)
        return 0
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        print(f"advisory Miri guard failure: {exc}", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
