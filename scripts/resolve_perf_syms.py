#!/usr/bin/env python3
"""Resolve stripped system-library symbols in `perf script` output and report trace quality.

`perf script` names frames from a DSO's on-disk symbol table only. System libraries on
Arch (libc, ld-linux, libstdc++, ...) ship stripped: their internal functions
(`_int_malloc`, `__memmove_avx512_unaligned_erms`, `__spawni_child`, ...) live only in the
separate debuginfo `.symtab`, so they appear as `[unknown]`. perf does NOT pull those names
from debuginfod even when the debug file is cached locally. This tool closes that gap:

  perf script -F +pid,+dsoff ...   →   frames carry `(<dso>+0x<offset>)`
  perf buildid-list                →   maps each DSO to its build-id
  debuginfod-find debuginfo <id>   →   locates the cached debug file (honours DEBUGINFOD_URLS)
  eu-addr2line / addr2line         →   offset → function name

Resolved frames have `[unknown]` replaced with the real name; the `+0x<offset>` suffix is
stripped from every DSO so the output stays in standard `perf script` form (Firefox Profiler
and inferno both parse it). Everything is best-effort: if a tool or debug file is missing the
frame is passed through unchanged.

A period-weighted quality summary is written to stderr so a caller can judge — without opening
the trace — whether the capture is dominated by real work or by startup/idle/subprocess noise.

Usage:
  perf script -i perf.data -F +pid,+dsoff --no-inline \
    | resolve_perf_syms.py --perf-data perf.data --target-comm end_to_end > perf.script
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
from collections import defaultdict

# A stack frame line, e.g. "\t    7f50.. [unknown] (/usr/lib/libc.so.6+0x104ce4)".
FRAME_RE = re.compile(r"^(\s+)([0-9a-fA-F]+)\s+(.*?)\s+\((.*)\)\s*$")
# A sample header line, e.g. "comm  1234/1234  987.65:  4200 cpu/cycles/Pu:".
HEADER_RE = re.compile(r"^(\S.*?)\s+\d+(?:/\d+)?\s+\d+\.\d+:\s+(\d+)\s+\S")
DSOFF_RE = re.compile(r"^(.*)\+0x[0-9a-fA-F]+$")


def tool(*names):
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    return None


def build_id_map(perf_data):
    """Map DSO path -> build-id from the trace itself (more correct than reading on-disk files)."""
    exe = tool("perf")
    if not exe or not perf_data:
        return {}
    try:
        out = subprocess.run(
            [exe, "buildid-list", "-i", perf_data],
            capture_output=True, text=True, check=False,
        ).stdout
    except OSError:
        return {}
    mapping = {}
    for line in out.splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2 and parts[1].startswith("/"):
            mapping[parts[1].strip()] = parts[0].strip()
    return mapping


class Resolver:
    """Lazily locates debug files and batch-resolves offsets per DSO."""

    def __init__(self, perf_data):
        self.buildids = build_id_map(perf_data)
        self.addr2line = tool("eu-addr2line", "addr2line")
        self.find = tool("debuginfod-find")
        self._debug_file = {}          # dso -> debug file path or None
        self._pending = defaultdict(set)   # dso -> {offset ints}
        self._resolved = {}            # (dso, offset) -> name

    def note(self, dso, off_hex):
        """Record an offset that needs resolving for a later batch pass."""
        if self._debug_file_for(dso) is None:
            return
        try:
            self._pending[dso].add(int(off_hex, 16))
        except ValueError:
            pass

    def _debug_file_for(self, dso):
        if dso in self._debug_file:
            return self._debug_file[dso]
        path = None
        bid = self.buildids.get(dso)
        if bid and self.find and self.addr2line:
            try:
                res = subprocess.run(
                    [self.find, "debuginfo", bid],
                    capture_output=True, text=True, check=False,
                )
                cand = res.stdout.strip()
                if cand and os.path.exists(cand):
                    path = cand
            except OSError:
                path = None
        self._debug_file[dso] = path
        return path

    def run_batches(self):
        """Resolve all noted offsets, one addr2line invocation per DSO."""
        for dso, offsets in self._pending.items():
            dbg = self._debug_file.get(dso)
            if not dbg or not offsets:
                continue
            ordered = sorted(offsets)
            args = [self.addr2line, "-f", "-e", dbg] + [hex(o) for o in ordered]
            try:
                out = subprocess.run(
                    args, capture_output=True, text=True, check=False,
                ).stdout.splitlines()
            except OSError:
                continue
            # eu-addr2line/addr2line with -f emit two lines per address: name, then file:line.
            names = [out[i] for i in range(0, len(out), 2)]
            for off, name in zip(ordered, names):
                name = name.strip()
                if name and name not in ("??", ""):
                    self._resolved[(dso, off)] = name

    def name_for(self, dso, off_hex):
        try:
            return self._resolved.get((dso, int(off_hex, 16)))
        except ValueError:
            return None


def split_dsoff(dso_field):
    """'/usr/lib/libc.so.6+0x104ce4' -> ('/usr/lib/libc.so.6', '104ce4'); else (field, None)."""
    m = DSOFF_RE.match(dso_field)
    if m:
        return m.group(1), dso_field[len(m.group(1)) + 3:]
    return dso_field, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--perf-data", default="")
    ap.add_argument("--target-comm", default="", help="process comm prefix considered 'signal'")
    ap.add_argument("--input", default="-")
    args = ap.parse_args()

    raw = sys.stdin.read() if args.input == "-" else open(args.input).read()
    lines = raw.splitlines()

    resolver = Resolver(args.perf_data)

    # Pass 1: discover every (dso, offset) behind an [unknown] frame so we can batch addr2line.
    for line in lines:
        m = FRAME_RE.match(line)
        if not m:
            continue
        sym, dso_field = m.group(3), m.group(4)
        if sym != "[unknown]":
            continue
        dso, off = split_dsoff(dso_field)
        if dso.startswith("/") and off is not None:
            resolver.note(dso, off)
    resolver.run_batches()

    # Pass 2: rewrite frames (strip +0x.. suffix; fill in resolved names) and gather stats.
    out_lines = []
    total_samples = empty_samples = 0
    total_period = 0
    unresolved_period = 0          # cycles whose leaf frame stayed [unknown]/kernel
    comm_period = defaultdict(int)
    cur_period = 0
    cur_comm = ""
    cur_leaf_unresolved = None

    def flush_sample():
        nonlocal unresolved_period
        if cur_leaf_unresolved:
            unresolved_period += cur_period

    for line in lines:
        h = HEADER_RE.match(line)
        if h:
            # finalize previous sample
            if cur_comm:
                flush_sample()
            cur_comm = h.group(1).strip()
            cur_period = int(h.group(2))
            total_samples += 1
            total_period += cur_period
            comm_period[cur_comm] += cur_period
            cur_leaf_unresolved = None
            out_lines.append(line)
            continue

        m = FRAME_RE.match(line)
        if not m:
            # blank line between samples => detect empty stacks (header followed by blank)
            if line.strip() == "" and out_lines and HEADER_RE.match(out_lines[-1] or ""):
                empty_samples += 1
                cur_leaf_unresolved = True   # no frame at all == unresolved leaf
            out_lines.append(line)
            continue

        indent, ip, sym, dso_field = m.groups()
        dso, off = split_dsoff(dso_field)
        new_sym = sym
        if sym == "[unknown]" and off is not None and dso.startswith("/"):
            r = resolver.name_for(dso, off)
            if r:
                new_sym = r
        # leaf frame (first frame after header) determines sample resolution
        if cur_leaf_unresolved is None:
            cur_leaf_unresolved = new_sym == "[unknown]"
        out_lines.append(f"{indent}{ip} {new_sym} ({dso})")

    if cur_comm:
        flush_sample()

    sys.stdout.write("\n".join(out_lines))
    if raw.endswith("\n"):
        sys.stdout.write("\n")

    # ---- quality summary (stderr) ----
    if total_period == 0:
        print("[quality] no samples captured", file=sys.stderr)
        return

    foreign_period = 0
    if args.target_comm:
        for comm, p in comm_period.items():
            if not comm.startswith(args.target_comm[:15]):  # perf truncates comm to 15 chars
                foreign_period += p

    def pct(x):
        return 100.0 * x / total_period

    print(f"[quality] samples={total_samples} "
          f"empty_stacks={empty_samples} ({100.0*empty_samples/total_samples:.0f}% of count)",
          file=sys.stderr)
    print(f"[quality] period-weighted: unresolved-leaf={pct(unresolved_period):.1f}%  "
          f"foreign-process={pct(foreign_period):.1f}%", file=sys.stderr)
    if len(comm_period) > 1:
        top = sorted(comm_period.items(), key=lambda kv: -kv[1])[:4]
        summary = ", ".join(f"{c}={pct(p):.0f}%" for c, p in top)
        print(f"[quality] processes by cycles: {summary}", file=sys.stderr)

    warnings = []
    if args.target_comm and pct(foreign_period) > 20:
        warnings.append(
            f"{pct(foreign_period):.0f}% of cycles are in OTHER processes (subprocess "
            f"pollution — e.g. a `dotnet build`); pre-build fixtures before recording")
    if pct(unresolved_period) > 40:
        warnings.append(
            f"{pct(unresolved_period):.0f}% of cycles have an unresolved leaf frame "
            f"(short/idle-dominated capture or missing debuginfo)")
    if total_samples < 200:
        warnings.append(
            f"only {total_samples} samples — too few for a meaningful flamegraph; "
            f"profile the `bench` target (steady-state), not a one-shot fixture")
    for w in warnings:
        print(f"[quality] WARNING: {w}", file=sys.stderr)
    if not warnings:
        print("[quality] OK: trace is dominated by resolved in-process work", file=sys.stderr)


if __name__ == "__main__":
    main()
