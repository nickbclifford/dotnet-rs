#!/usr/bin/env python3
"""Capture a bounded CI phase without turning failed or unfinished work green.

Example: python3 scripts/ci_diagnostics.py run --dir "$RUNNER_TEMP/ci-diagnostics/job" \
    --phase execute --case value --timeout 600 --toolchain nightly-2026-05-27 \
    --features no-features --cache unknown -- cargo test ...

Each phase/case gets a .start.json, .log and .status.json. IDs are restricted to
artifact-safe filenames; an existing start marker is never overwritten. Optional
--require checks literal output after a successful child (for exact count guards).
Use a separate guard phase for more complex manifest/count checks. `finalize`
can record an interrupted phase if a runner reaches an always() step but the
wrapper did not write its own status; it never treats missing status as success.
This helper does not replace the pointer script's exact single-pass guard.
"""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import time


SAFE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}\Z")
UB = re.compile(r"error:\s+Undefined Behavior\b", re.IGNORECASE)
TEST_FAILURE = re.compile(r"panicked at|assertion .*failed|test .* \.\.\. FAILED", re.IGNORECASE)
GUARD_FAILURE = re.compile(
    r"zero-ignored-or-unverified-count|control discovery/ignore guard failed|"
    r"expected exactly \d+ passing controls|Replay validation failed: (?:"
    r"expected exactly one |no tracked corpus files|corpus census mismatch|"
    r"corpus file differs from git index|corpus changed during replay|"
    r"reported file/seed census|replay summary events out of order|"
    r"inconsistent or insufficient execution counts|libFuzzer loaded a different corpus path|"
    r"non-regular or empty corpus file|non-regular or unmerged tracked entry|"
    r"empty corpus (?:file|census)|symlink in corpus|corpus directory must match target)",
    re.IGNORECASE,
)


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def safe_id(value):
    if not SAFE_ID.fullmatch(value) or value in (".", ".."):
        raise argparse.ArgumentTypeError("use 1-128 ASCII letters, digits, '.', '_' or '-' (no path separators)")
    return value


def positive_seconds(value):
    try:
        result = float(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("timeout must be a positive finite number of seconds") from exc
    if not 0 < result < float("inf"):
        raise argparse.ArgumentTypeError("timeout must be a positive finite number of seconds")
    return result


def paths(directory, phase, case):
    stem = f"{phase}--{case}"
    directory = Path(directory)
    return tuple(directory / (stem + suffix) for suffix in (".start.json", ".log", ".status.json"))


def write_status(path, value):
    # Atomic replacement keeps a partial write from masquerading as a final result.
    temp = path.with_name(path.name + f".{os.getpid()}.tmp")
    try:
        with temp.open("x", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2, ensure_ascii=True)
            stream.write("\n")
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


def sha():
    if os.environ.get("GITHUB_SHA"):
        return os.environ["GITHUB_SHA"]
    try:
        return subprocess.check_output(["git", "rev-parse", "HEAD"], stderr=subprocess.DEVNULL, text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def classify(phase, rc, text, deadline, cancelled, missing):
    if deadline:
        return "budget-exhaustion"
    if cancelled:
        return "cancelled-or-unknown"
    if rc == 0 and not missing:
        return "success"
    if UB.search(text):
        return "miri-ub"
    if missing or phase == "guard" or GUARD_FAILURE.search(text):
        return "validation-guard-failure"
    if rc < 0:
        return "cancelled-or-unknown"
    if phase in ("setup", "compile"):
        return "setup-error"
    if TEST_FAILURE.search(text):
        return "test-failure"
    return "unclassified-process-error"


def run(args):
    if not args.command:
        raise ValueError("a command after -- is required")
    directory = Path(args.dir)
    directory.mkdir(parents=True, exist_ok=True)
    start_path, log_path, status_path = paths(directory, args.phase, args.case)
    # x avoids accidental log loss when two cases collide or a job is retried.
    start = {
        "version": 1, "phase": args.phase, "subphase": args.subphase,
        "case": args.case, "started_utc": utc_now(), "sha": sha(),
        "command": args.command, "cwd": os.getcwd(), "timeout_seconds": args.timeout,
        "features": args.features, "miriflags": os.environ.get("MIRIFLAGS", ""),
        "toolchain": args.toolchain, "cache": args.cache, "required_output": args.require,
    }
    with start_path.open("x", encoding="utf-8") as stream:
        json.dump(start, stream, indent=2, ensure_ascii=True)
        stream.write("\n")
    # Status/log must not silently clobber evidence from a previous incomplete run.
    if status_path.exists() or log_path.exists():
        raise FileExistsError("status or log already exists for phase/case")

    received_signal = None
    def on_signal(signum, _frame):
        nonlocal received_signal
        if received_signal is None:
            received_signal = signum

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGTERM, signal.SIGINT)}
    started = time.monotonic()
    deadline = False
    terminating_at = None
    process = None
    rc = 127
    io_error = None
    try:
        with log_path.open("xb") as log:
            try:
                process = subprocess.Popen(args.command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                           start_new_session=True, bufsize=0)
            except OSError as exc:
                message = f"ci-diagnostics: could not start command: {exc}\n".encode()
                log.write(message)
                log.flush()
                sys.stderr.buffer.write(message)
                sys.stderr.buffer.flush()
            else:
                os.set_blocking(process.stdout.fileno(), False)
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    while selector.get_map() or process.poll() is None:
                        now = time.monotonic()
                        # Only our own elapsed deadline is evidence of budget exhaustion.
                        if terminating_at is None and received_signal is None and now - started >= args.timeout:
                            deadline = True
                        if terminating_at is None and (deadline or received_signal is not None):
                            terminating_at = now
                            try:
                                os.killpg(process.pid, signal.SIGTERM if deadline else received_signal)
                            except ProcessLookupError:
                                pass
                        if terminating_at is not None and now - terminating_at >= 2:
                            try:
                                os.killpg(process.pid, signal.SIGKILL)
                            except ProcessLookupError:
                                pass
                        for key, _ in selector.select(0.05):
                            chunk = os.read(key.fd, 65536)
                            if not chunk:
                                selector.unregister(key.fileobj)
                                continue
                            log.write(chunk)
                            log.flush()
                            try:
                                sys.stdout.buffer.write(chunk)
                                sys.stdout.buffer.flush()
                            except (BrokenPipeError, OSError) as exc:
                                # Preserve the on-disk log, but never report a clean run.
                                io_error = str(exc)
                process.stdout.close()
                rc = process.wait()
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    text = log_path.read_text(encoding="utf-8", errors="replace")
    missing = [item for item in args.require if item not in text] if rc == 0 else []
    classification = classify(args.phase, rc, text, deadline, received_signal, missing)
    if io_error and classification == "success":
        classification = "unclassified-process-error"
    exit_code = (124 if deadline else 128 + received_signal if received_signal else
                 128 - rc if rc < 0 else rc if rc else 1 if missing or io_error else 0)
    status = {
        "version": 1, "phase": args.phase, "subphase": args.subphase, "case": args.case,
        "ended_utc": utc_now(), "elapsed_seconds": round(time.monotonic() - started, 3),
        "classification": classification, "child_returncode": rc, "exit_code": exit_code,
        "deadline_expired": deadline, "received_signal": received_signal,
        "missing_required_output": missing, "output_error": io_error,
        "log": log_path.name,
    }
    write_status(status_path, status)
    print(f"ci-diagnostics: {args.phase}/{args.case}: {classification} (exit={exit_code}; log={log_path})", file=sys.stderr)
    return exit_code


def finalize(args):
    start_path, log_path, status_path = paths(args.dir, args.phase, args.case)
    if status_path.exists():
        return 0
    if not start_path.exists():
        print("ci-diagnostics: no start marker; no phase result fabricated", file=sys.stderr)
        return 0
    start = json.loads(start_path.read_text(encoding="utf-8"))
    status = {
        "version": 1, "phase": args.phase, "subphase": start.get("subphase", ""), "case": args.case,
        "ended_utc": utc_now(), "classification": "cancelled-or-unknown" if args.outcome == "cancelled" else "missing-status",
        "step_outcome": args.outcome, "child_returncode": None, "exit_code": None,
        "deadline_expired": False, "received_signal": None, "log": log_path.name,
        "finalized_from_start_marker": True,
    }
    write_status(status_path, status)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    run_parser = sub.add_parser("run")
    final_parser = sub.add_parser("finalize")
    for item in (run_parser, final_parser):
        item.add_argument("--dir", required=True)
        item.add_argument("--phase", choices=("setup", "compile", "execute", "guard"), required=True)
        item.add_argument("--case", type=safe_id, required=True)
    run_parser.add_argument("--subphase", default="")
    run_parser.add_argument("--timeout", type=positive_seconds, required=True)
    run_parser.add_argument("--features", default="unknown")
    run_parser.add_argument("--toolchain", default="unknown")
    run_parser.add_argument("--cache", default="unknown")
    run_parser.add_argument("--require", action="append", default=[])
    run_parser.add_argument("command", nargs=argparse.REMAINDER)
    final_parser.add_argument("--outcome", choices=("success", "failure", "cancelled", "skipped", "unknown"), required=True)
    args = parser.parse_args()
    if args.action == "run" and args.command and args.command[0] == "--":
        args.command = args.command[1:]
    try:
        return run(args) if args.action == "run" else finalize(args)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"ci-diagnostics: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
