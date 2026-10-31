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

CI defaults come from CI_DIAG_DIR, CI_DIAG_DEADLINE (epoch seconds), CI_DIAG_FEATURES,
CI_DIAG_TOOLCHAIN and CI_DIAG_CACHE. A deadline caps every phase timeout so an inner
budget always expires before the job timeout and leaves time to summarize/upload; a
phase started after it is recorded as budget exhaustion without running. `summarize`
finalizes any started phase lacking a status and prints a Markdown table; it always
exits 0 so it can never change a job's result.
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


def descendant_groups(root):
    """Process groups of root's descendants (Linux /proc; empty elsewhere).

    GNU timeout moves itself and its command into a new process group, so signalling
    only the wrapper's session would orphan e.g. the pointer script's Miri children.
    """
    children = {}
    try:
        entries = os.listdir("/proc")
    except OSError:
        return set()
    for entry in entries:
        if not entry.isdigit():
            continue
        try:
            fields = Path("/proc", entry, "stat").read_text().rsplit(")", 1)[1].split()
            children.setdefault(int(fields[1]), []).append((int(entry), int(fields[2])))
        except (OSError, IndexError, ValueError):
            continue
    groups, stack = set(), [root]
    while stack:
        for pid, pgid in children.get(stack.pop(), ()):
            groups.add(pgid)
            stack.append(pid)
    return groups - {os.getpgrp()}


def signal_groups(groups, signum):
    for group in groups:
        try:
            os.killpg(group, signum)
        except (ProcessLookupError, PermissionError):
            pass


def run(args):
    if not args.command:
        raise ValueError("a command after -- is required")
    limit = args.timeout
    if args.deadline_epoch is not None:
        limit = min(limit, args.deadline_epoch - time.time())
    directory = Path(args.dir)
    directory.mkdir(parents=True, exist_ok=True)
    start_path, log_path, status_path = paths(directory, args.phase, args.case)
    # x avoids accidental log loss when two cases collide or a job is retried.
    start = {
        "version": 1, "phase": args.phase, "subphase": args.subphase,
        "case": args.case, "started_utc": utc_now(), "sha": sha(),
        "command": args.command, "cwd": os.getcwd(), "timeout_seconds": args.timeout,
        "deadline_epoch": args.deadline_epoch, "effective_timeout_seconds": round(max(limit, 0), 3),
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
    groups = set()
    process = None
    rc = 127
    io_error = None
    try:
        with log_path.open("xb") as log:
            if limit <= 0:
                # The job budget is already spent: running would only eat the upload window.
                deadline = True
                rc = 124
                message = b"ci-diagnostics: job deadline already passed; command not started\n"
                log.write(message)
                sys.stderr.buffer.write(message)
                sys.stderr.buffer.flush()
            else:
                try:
                    process = subprocess.Popen(args.command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                               start_new_session=True, bufsize=0)
                except OSError as exc:
                    message = f"ci-diagnostics: could not start command: {exc}\n".encode()
                    log.write(message)
                    log.flush()
                    sys.stderr.buffer.write(message)
                    sys.stderr.buffer.flush()
            if process is not None:
                os.set_blocking(process.stdout.fileno(), False)
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    while selector.get_map() or process.poll() is None:
                        now = time.monotonic()
                        # Only our own elapsed deadline is evidence of budget exhaustion.
                        if terminating_at is None and received_signal is None and now - started >= limit:
                            deadline = True
                        if terminating_at is None and (deadline or received_signal is not None):
                            terminating_at = now
                            # Snapshot before signalling: orphans lose their ancestry.
                            groups = {process.pid} | descendant_groups(process.pid)
                            signal_groups(groups, signal.SIGTERM if deadline else received_signal)
                        if terminating_at is not None and now - terminating_at >= 2:
                            signal_groups(groups | descendant_groups(process.pid), signal.SIGKILL)
                        if terminating_at is not None and now - terminating_at >= 5 and process.poll() is not None:
                            break  # An unkillable holder of the pipe must not hang the summary.
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


def finalize_one(directory, phase, case, outcome):
    start_path, log_path, status_path = paths(directory, phase, case)
    if status_path.exists():
        return False
    if not start_path.exists():
        print("ci-diagnostics: no start marker; no phase result fabricated", file=sys.stderr)
        return False
    start = json.loads(start_path.read_text(encoding="utf-8"))
    status = {
        "version": 1, "phase": phase, "subphase": start.get("subphase", ""), "case": case,
        "ended_utc": utc_now(), "classification": "cancelled-or-unknown" if outcome == "cancelled" else "missing-status",
        "step_outcome": outcome, "child_returncode": None, "exit_code": None,
        "deadline_expired": False, "received_signal": None, "log": log_path.name,
        "finalized_from_start_marker": True,
    }
    write_status(status_path, status)
    return True


def finalize(args):
    finalize_one(args.dir, args.phase, args.case, args.outcome)
    return 0


def summarize(args):
    directory = Path(args.dir)
    rows, errors = [], []
    for start_path in sorted(directory.glob("*.start.json")) if directory.is_dir() else ():
        try:
            start = json.loads(start_path.read_text(encoding="utf-8"))
            phase, case = start["phase"], start["case"]
            finalize_one(directory, phase, case, args.outcome)
            status = json.loads(paths(directory, phase, case)[2].read_text(encoding="utf-8"))
            rows.append((start, status))
        except (OSError, KeyError, TypeError, ValueError) as exc:
            errors.append(f"{start_path.name}: {exc}")
    rows.sort(key=lambda row: row[0].get("started_utc", ""))
    failed = [status for _, status in rows if status.get("classification") != "success"]
    kind = "advisory" if args.advisory else "blocking"
    lines = [f"### CI diagnostics: {args.title}", "",
             f"Job status at summary: `{args.outcome}` ({kind}). Logs, start and status metadata are in this job's "
             "`ci-diagnostics-*` artifact."]
    if not rows:
        lines.append("No phase evidence was recorded (the job stopped before its first wrapped phase). "
                     "Fall back to `gh run view RUN --job JOB --log` and the check-run annotations.")
    elif failed and args.advisory:
        lines.append(f"**Advisory diagnostic failure:** {len(failed)} phase(s) did not succeed; "
                     "this is not a successful completion even though the job is allowed to fail.")
    elif failed:
        lines.append(f"**Blocking failure:** {len(failed)} phase(s) did not succeed.")
    else:
        lines.append(f"All {len(rows)} recorded phase(s) succeeded.")
    if rows:
        lines += ["", "| Phase | Subphase | Case | Result | Elapsed (s) | Budget (s) | Exit | Log |",
                  "|---|---|---|---|---|---|---|---|"]
        for start, status in rows:
            cells = (start.get("phase"), start.get("subphase") or "-", start.get("case"),
                     status.get("classification"), status.get("elapsed_seconds", "-"),
                     start.get("effective_timeout_seconds", start.get("timeout_seconds", "-")),
                     status.get("exit_code"), status.get("log"))
            lines.append("| " + " | ".join("-" if cell is None else str(cell).replace("|", "\\|") for cell in cells) + " |")
    if errors:
        lines += ["", "Unreadable evidence: " + "; ".join(errors)]
    text = "\n".join(lines) + "\n"
    try:
        if directory.is_dir():
            (directory / "summary.md").write_text(text, encoding="utf-8")
    except OSError as exc:
        text += f"\nCould not write summary.md: {exc}\n"
    sys.stdout.write(text)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    run_parser = sub.add_parser("run")
    final_parser = sub.add_parser("finalize")
    summary_parser = sub.add_parser("summarize")
    env_dir = os.environ.get("CI_DIAG_DIR")
    for item in (run_parser, final_parser, summary_parser):
        item.add_argument("--dir", default=env_dir, required=not env_dir)
    for item in (run_parser, final_parser):
        item.add_argument("--phase", choices=("setup", "compile", "execute", "guard"), required=True)
        item.add_argument("--case", type=safe_id, required=True)
    run_parser.add_argument("--subphase", default="")
    run_parser.add_argument("--timeout", type=positive_seconds, required=True)
    env_deadline = os.environ.get("CI_DIAG_DEADLINE")
    run_parser.add_argument("--deadline-epoch", type=float, default=float(env_deadline) if env_deadline else None)
    run_parser.add_argument("--features", default=os.environ.get("CI_DIAG_FEATURES", "unknown"))
    run_parser.add_argument("--toolchain", default=os.environ.get("CI_DIAG_TOOLCHAIN", "unknown"))
    run_parser.add_argument("--cache", default=os.environ.get("CI_DIAG_CACHE", "unknown"))
    run_parser.add_argument("--require", action="append", default=[])
    run_parser.add_argument("command", nargs=argparse.REMAINDER)
    for item in (final_parser, summary_parser):
        item.add_argument("--outcome", choices=("success", "failure", "cancelled", "skipped", "unknown"), required=True)
    summary_parser.add_argument("--title", required=True)
    summary_parser.add_argument("--advisory", action="store_true")
    args = parser.parse_args()
    if args.action == "run" and args.command and args.command[0] == "--":
        args.command = args.command[1:]
    try:
        return {"run": run, "finalize": finalize, "summarize": summarize}[args.action](args)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"ci-diagnostics: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
