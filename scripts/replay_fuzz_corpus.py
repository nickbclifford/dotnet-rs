#!/usr/bin/env python3
"""Replay exactly the tracked, nonempty corpus and verify libFuzzer's execution summary."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys


class ReplayError(Exception):
    pass


def inventory(root, workdir, corpus, target):
    root = Path(root).resolve()
    workdir = root / workdir
    corpus_path = workdir / corpus
    # Reject escaping and symlinked paths, including parent directories.
    for path in (workdir, corpus_path):
        try:
            relative = path.relative_to(root)
        except ValueError as exc:
            raise ReplayError(f"path outside repository: {path}") from exc
        current = root
        for part in relative.parts:
            if part in ("..", "."):
                raise ReplayError(f"non-canonical path: {path}")
            current /= part
            if current.is_symlink():
                raise ReplayError(f"symlink in corpus path: {current}")
        if not path.is_dir():
            raise ReplayError(f"directory missing: {path}")
    if corpus_path.name != target:
        raise ReplayError("corpus directory must match target")
    rel = corpus_path.relative_to(root).as_posix()
    object_format = subprocess.check_output(["git", "rev-parse", "--show-object-format"], cwd=root, text=True).strip()
    result = subprocess.run(
        ["git", "ls-files", "--stage", "-z", "--", f":(literal){rel}/"],
        cwd=root, check=True, stdout=subprocess.PIPE,
    )
    tracked = set()
    for entry in result.stdout.split(b"\0"):
        if not entry:
            continue
        metadata, filename = entry.split(b"\t", 1)
        mode, oid, stage = metadata.split()
        if mode not in (b"100644", b"100755") or stage != b"0":
            raise ReplayError(f"non-regular or unmerged tracked entry: {os.fsdecode(filename)}")
        tracked.add((os.fsdecode(filename), oid.decode("ascii")))
    if not tracked:
        raise ReplayError("no tracked corpus files")
    tracked = dict(tracked)
    found = set()
    manifest = []
    for directory, dirs, files in os.walk(corpus_path, followlinks=False):
        for name in dirs:
            path = Path(directory) / name
            if path.is_symlink():
                raise ReplayError(f"symlink in corpus: {path}")
        for name in files:
            path = Path(directory) / name
            info = path.lstat()
            if not stat.S_ISREG(info.st_mode) or info.st_size == 0:
                raise ReplayError(f"non-regular or empty corpus file: {path}")
            relative = path.relative_to(root).as_posix()
            data = path.read_bytes()
            if not data:
                raise ReplayError(f"empty corpus file: {path}")
            found.add(relative)
            blob = hashlib.new(object_format, f"blob {len(data)}\0".encode() + data).hexdigest()
            if relative in tracked and blob != tracked[relative]:
                raise ReplayError(f"corpus file differs from git index: {relative}")
            manifest.append({"path": relative, "bytes": len(data),
                             "sha256": hashlib.sha256(data).hexdigest()})
    if found != tracked.keys():
        raise ReplayError(f"corpus census mismatch; missing={sorted(tracked.keys() - found)!r}, unexpected={sorted(found - tracked.keys())!r}")
    return sorted(manifest, key=lambda entry: entry["path"])


def verify_log(log, count, workdir, corpus):
    if count <= 0:
        raise ReplayError("empty corpus census")
    text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", log)
    patterns = (
        r"^INFO:\s+(\d+) files found in (.+)\s*$",
        r"^INFO: seed corpus: files:\s*(\d+)\b.*$",
        r"^#(\d+)\s+INITED\b.*$",
        r"^#(\d+)\s+DONE\b.*$",
        r"^Done (\d+) runs in \d+ second\(s\)\s*$",
    )
    events = []
    for pattern in patterns:
        matches = list(re.finditer(pattern, text, re.MULTILINE))
        if len(matches) != 1:
            raise ReplayError(f"expected exactly one {pattern!r}, found {len(matches)}")
        events.append(matches[0])
    if any(a.start() >= b.start() for a, b in zip(events, events[1:])):
        raise ReplayError("replay summary events out of order")
    if int(events[0][1]) != count or int(events[1][1]) != count:
        raise ReplayError("reported file/seed census differs from tracked corpus")
    reported = Path(events[0][2].strip())
    if not reported.is_absolute():
        reported = Path(workdir) / reported
    if reported.resolve() != (Path(workdir) / corpus).resolve():
        raise ReplayError("libFuzzer loaded a different corpus path")
    runs = [int(event[1]) for event in events[2:]]
    if len(set(runs)) != 1 or runs[0] < count + 1:
        raise ReplayError(f"inconsistent or insufficient execution counts: {runs}; need at least {count + 1}")
    return runs[0]


def replay(root, workdir, corpus, target, output_dir):
    output_dir = Path(output_dir).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    log_path = output_dir / f"{target}.log"
    manifest_path = output_dir / f"{target}.manifest.json"
    # Create a log before validation so even precondition failures leave evidence.
    with log_path.open("wb") as log:
        try:
            if manifest_path.exists():
                manifest_path.unlink()
            manifest = inventory(root, workdir, corpus, target)
            manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
            print(f"Corpus manifest: {manifest_path} ({len(manifest)} tracked files)", flush=True)
            print(json.dumps(manifest, indent=2), flush=True)
            cmd = ["cargo", "+nightly-2026-05-27", "fuzz", "run", target, corpus, "--", "-runs=0"]
            print(f"Replay command: {cmd!r}; log: {log_path}", flush=True)
            process = subprocess.Popen(cmd, cwd=Path(root) / workdir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            while chunk := process.stdout.read1(65536):
                log.write(chunk)
                log.flush()
                sys.stdout.buffer.write(chunk)
                sys.stdout.buffer.flush()
            process.stdout.close()
            status = process.wait()
            if status:
                raise ReplayError(f"replay child exited {status}")
            if inventory(root, workdir, corpus, target) != manifest:
                raise ReplayError("corpus changed during replay")
            runs = verify_log(log_path.read_text(errors="replace"), len(manifest), Path(root) / workdir, corpus)
            print(f"Verified {len(manifest)} tracked files and {runs} executions", flush=True)
        except (ReplayError, OSError, subprocess.CalledProcessError) as exc:
            message = f"Replay validation failed: {exc}\n"
            log.write(message.encode())
            log.flush()
            print(message, file=sys.stderr, end="", flush=True)
            return 1
    return 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--workdir", required=True)
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    return replay(root, args.workdir, args.corpus, args.target, args.output_dir)


if __name__ == "__main__":
    sys.exit(main())
