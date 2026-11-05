#!/usr/bin/env python3
"""Select which deep CI lanes must run for a set of changed paths.

Policy (docs/CI.md, "Selection and merge policy"): ambiguity always selects MORE.
An unrecognized path, a missing/zero base, a git error, an empty diff or any
exception selects every deep lane (mode=full). The script never exits non-zero
on a selection problem; the only non-zero exits are argument errors or a failed
output write, and in both cases ci.yml's select-deep job falls back to full.

Inputs (exactly one):
  --full                      select everything (nightly / manual full sweep)
  --paths-from FILE           newline-separated changed paths (tests)
  --base REV --head REV       diff the two revisions; --range selects the form:
        pr    (default)  git diff base...head  merge-base diff: what the branch
                         introduces, ignoring commits that landed on the base
                         after the branch forked. Correct for pull_request and
                         for workflow_dispatch against origin/main.
        push             git diff base..head   direct diff: everything the push
                         moved the branch by. `github.event.before` is the old
                         branch tip, so a merge-base would hide force-pushed or
                         merged-in changes.

Outputs go to $GITHUB_OUTPUT (or stdout when unset) and a per-lane summary to
$GITHUB_STEP_SUMMARY: mode, cross_arena, pointer_controls, pointer_mode,
fuzz_any, fuzz_matrix (JSON, the include entries ci.yml used to hold statically),
reason_cross_arena, reason_pointer_controls, reason_fuzz.

pointer_mode chooses how the selected pointer-control lane runs: "singleton" in
every full sweep (the 37 controls as one Miri process each, per configuration) and
"batched" for a selected push/PR run (a few batches per configuration, with the
singleton jobs re-run only to attribute a batch failure). Whenever mode is "full",
including every fail-safe fallback, pointer_mode is "singleton".
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

REPO_ROOT = Path(__file__).resolve().parents[1]

# Lanes (selection tokens).
CROSS_ARENA = "cross_arena"
POINTER = "pointer_controls"
FUZZ_VALUE = "fuzz_value"  # the three crates/dotnet-value/fuzz targets
FUZZ_EXECUTOR = "fuzz_executor"  # crates/dotnet-vm/fuzz
ALL = frozenset({CROSS_ARENA, POINTER, FUZZ_VALUE, FUZZ_EXECUTOR})
NOTHING = frozenset()
VM_TIER = frozenset({CROSS_ARENA, POINTER, FUZZ_EXECUTOR})

# Exact include entries ci.yml's fuzz-corpus-replay matrix used before selection.
# Keep every field identical; test_ci_select.py pins these and the ci.yml fallback.
FUZZ_ENTRIES = {
    FUZZ_VALUE: [
        {
            "target": "fuzz_managed_ptr_roundtrip",
            "working-directory": "crates/dotnet-value",
            "corpus": "fuzz/corpus/fuzz_managed_ptr_roundtrip",
            "fuzz-workspace": "crates/dotnet-value/fuzz",
        },
        {
            "target": "fuzz_managed_ptr_offset",
            "working-directory": "crates/dotnet-value",
            "corpus": "fuzz/corpus/fuzz_managed_ptr_offset",
            "fuzz-workspace": "crates/dotnet-value/fuzz",
        },
        {
            "target": "fuzz_raw_memory_access",
            "working-directory": "crates/dotnet-value",
            "corpus": "fuzz/corpus/fuzz_raw_memory_access",
            "fuzz-workspace": "crates/dotnet-value/fuzz",
        },
    ],
    FUZZ_EXECUTOR: [
        {
            "target": "fuzz_executor",
            "working-directory": "crates/dotnet-vm/fuzz",
            "corpus": "corpus/fuzz_executor",
            "fuzz-workspace": "crates/dotnet-vm/fuzz",
        },
    ],
}

# Path closures, hardcoded for the selector and re-derived from the Cargo.toml
# files by test_ci_select.py (which fails if this table drifts).
#   VALUE_CLOSURE: crates the dotnet-value fuzz workspace builds.
#   VM_CLOSURE: crates the Miri lanes and the dotnet-vm fuzz workspace build.
VALUE_CLOSURE = frozenset({"dotnet-value", "dotnet-types", "dotnet-utils", "dotnet-metrics"})
VM_CLOSURE = VALUE_CLOSURE | frozenset(
    {
        "dotnet-vm",
        "dotnet-assemblies",
        "dotnet-build-tools",
        "dotnet-exceptions",
        "dotnet-intrinsics-core",
        "dotnet-intrinsics-delegates",
        "dotnet-intrinsics-reflection",
        "dotnet-intrinsics-span",
        "dotnet-intrinsics-string",
        "dotnet-intrinsics-threading",
        "dotnet-intrinsics-unsafe",
        "dotnet-macros",
        "dotnet-macros-core",
        "dotnet-pinvoke",
        "dotnet-runtime-memory",
        "dotnet-runtime-resolver",
        "dotnet-simd",
        "dotnet-tracer",
        "dotnet-vm-data",
        "dotnet-vm-ops",
    }
)
# Workspace crates that no deep lane builds; their native tests stay in fast-core.
NON_DEEP_CRATES = frozenset({"dotnet-cli", "dotnet-benchmarks"})

# Root-level files whose change can alter any build or lint outcome.
ROOT_BUILD_CONFIG = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "rust-toolchain",
        "clippy.toml",
        ".clippy.toml",
        "rustfmt.toml",
        ".rustfmt.toml",
        "build.rs",
    }
)
# Scripts the deep lanes invoke (directly, via ci_diagnostics.py, or as a guard
# step of the lane's job). pointer_provenance.sh and ci_diagnostics.py import or
# call no other repo file; replay_fuzz_corpus.py only imports the stdlib.
DEEP_SCRIPTS = frozenset(
    {
        "scripts/ci_diagnostics.py",
        "scripts/pointer_provenance.sh",
        "scripts/replay_fuzz_corpus.py",
        "scripts/tests/test_replay_fuzz_corpus.py",
        "scripts/ci_select.py",
    }
)
# Non-deep workflow files (separate advisory workflows).
OTHER_WORKFLOWS = frozenset({"fuzz.yml", "miri.yml", "valgrind.yml"})
# Root files only used by local development or the advisory Valgrind workflow.
LOCAL_ONLY_ROOT = frozenset({"valgrind.supp", "check.sh", "bench.fish"})

LITERAL_MD = re.compile(r'"([^"\n]*\.md)"')


def _strip_rel(literal):
    parts = [p for p in literal.replace("\\", "/").split("/") if p not in ("", ".", "..")]
    return "/".join(parts)


def md_references(root):
    """Map stripped `.md` string-literal suffix -> list of Rust files that mention it.

    Any `include_str!("../README.md")`, `#[doc = include_str!(..)]` or
    `concat!(env!("CARGO_MANIFEST_DIR"), "/x.md")` style reference contains the
    file name as a string literal; matching by path suffix is deliberately broad.
    """
    refs = {}
    for top in ("crates", "xtask"):
        base = Path(root) / top
        if not base.is_dir():
            continue
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = [d for d in dirnames if d not in ("target", ".git")]
            for name in filenames:
                if not name.endswith(".rs"):
                    continue
                path = Path(dirpath) / name
                try:
                    text = path.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    continue
                rel = path.relative_to(root).as_posix()
                for lit in LITERAL_MD.findall(text):
                    key = _strip_rel(lit)
                    if key:
                        refs.setdefault(key, []).append(rel)
    return refs


def is_docs_path(path):
    """Pure documentation paths (before the include_str! override)."""
    p = PurePosixPath(path)
    name = p.name
    return (
        path.startswith("docs/")
        or path.startswith("experiments/")
        or name.endswith(".md")
        or name.startswith("LICENSE")
        or name.startswith("COPYING")
        or name.startswith("NOTICE")
    )


def classify(path, refs):
    """Return (selection set, family label, note) for one changed path."""
    path = path.strip().replace("\\", "/")
    if path.startswith("./"):
        path = path[2:]
    if not path:
        return ALL, "unrecognized", "empty path"

    if path.endswith(".md"):
        users = [
            rel
            for key, rels in refs.items()
            if path == key or path.endswith("/" + key)
            for rel in rels
        ]
        if users:
            sel = set()
            for user in users:
                sel |= classify(user, {})[0]
            return frozenset(sel), "code-referenced doc", "included by %s" % users[0]
    if is_docs_path(path):
        return NOTHING, "docs", "documentation only"

    parts = path.split("/")
    if parts[0] == "crates":
        if len(parts) < 3:
            return ALL, "unrecognized", "file directly under crates/"
        crate = parts[1]
        if crate == "dotnet-value" and parts[2] == "fuzz":
            return ALL, "value fuzz", "dotnet-value fuzz workspace (also in the VM closure)"
        if crate == "dotnet-vm" and parts[2] == "fuzz":
            return VM_TIER, "vm fuzz", "dotnet-vm fuzz workspace"
        if crate in VALUE_CLOSURE:
            return ALL, "value closure", "crate %s is in the dotnet-value fuzz and VM closures" % crate
        if crate in VM_CLOSURE:
            return VM_TIER, "vm closure", "crate %s is in the dotnet-vm closure" % crate
        if crate in NON_DEEP_CRATES:
            return NOTHING, "native-only crate", "crate %s is not built by any deep lane" % crate
        return ALL, "unrecognized", "unknown crate directory %s" % crate

    if path in ROOT_BUILD_CONFIG or path.endswith("/build.rs"):
        return ALL, "root build config", "workspace build configuration"
    if parts[0] == "xtask" or parts[0] == ".cargo":
        return ALL, "build tooling", "%s feeds every build" % parts[0]
    if parts[0] == ".config":
        return ALL, "test runner config", ".config/nextest.toml is used by pointer shard discovery"

    if path == ".github/workflows/ci.yml":
        return ALL, "ci.yml", "the workflow defines the deep lanes"
    if parts[0] == ".github":
        if len(parts) == 3 and parts[1] == "workflows" and parts[2] in OTHER_WORKFLOWS:
            return NOTHING, "other workflow", "separate advisory workflow"
        if parts[1:2] in (["actions"], ["workflows"]):
            return ALL, "ci infrastructure", "unlisted workflow or composite action"
        return NOTHING, "github metadata", ".github file no deep lane reads"
    if parts[0] == "scripts":
        if path in DEEP_SCRIPTS:
            return ALL, "deep-lane script", "script invoked by a deep lane"
        return NOTHING, "other script", "script no deep lane invokes"
    if len(parts) == 1 and path in LOCAL_ONLY_ROOT:
        return NOTHING, "local tooling", "not used by a deep lane"

    return ALL, "unrecognized", "unrecognized path"


def select(paths, root=REPO_ROOT):
    refs = md_references(root)
    sel_reasons = {}  # token -> (path, family, note) of first selecting path
    families = {}
    total = set()
    for path in paths:
        tokens, family, note = classify(path, refs)
        families[family] = families.get(family, 0) + 1
        for tok in tokens:
            sel_reasons.setdefault(tok, (path, family, note))
        total |= tokens
    return total, sel_reasons, families


def _why(tokens, sel_reasons, families, n_paths):
    """One-line reason covering the given selection tokens."""
    for tok in sorted(tokens):
        if tok in sel_reasons:
            path, family, note = sel_reasons[tok]
            return "selected by %s (%s: %s)" % (path, family, note)
    fams = ", ".join("%s x%d" % kv for kv in sorted(families.items())) or "none"
    return "not selected: none of %d changed paths affect this lane (%s)" % (n_paths, fams)


def build_outputs(mode, tokens, why):
    """why: callable(lane_tokens) -> reason string, or a plain string for full mode."""
    lane_tokens = {
        "cross_arena": {CROSS_ARENA},
        "pointer_controls": {POINTER},
        "fuzz": {FUZZ_VALUE, FUZZ_EXECUTOR},
    }
    reason = (lambda t: why) if isinstance(why, str) else why
    include = []
    for tok in (FUZZ_VALUE, FUZZ_EXECUTOR):
        if tok in tokens:
            include.extend(FUZZ_ENTRIES[tok])
    out = {
        "mode": mode,
        "cross_arena": "true" if CROSS_ARENA in tokens else "false",
        "pointer_controls": "true" if POINTER in tokens else "false",
        "pointer_mode": "singleton" if mode == "full" else "batched",
        "fuzz_any": "true" if include else "false",
        "fuzz_matrix": json.dumps({"include": include}, separators=(",", ":")),
        "reason_cross_arena": reason(lane_tokens["cross_arena"]),
        "reason_pointer_controls": reason(lane_tokens["pointer_controls"]),
        "reason_fuzz": reason(lane_tokens["fuzz"]),
    }
    for key in out:
        out[key] = out[key].replace("\r", " ").replace("\n", " ")
    return out


def full_outputs(why):
    return build_outputs("full", ALL, "full sweep: " + why)


class GitError(Exception):
    pass


def git_changed_paths(base, head, range_kind, cwd):
    if not base or set(base) <= {"0"}:
        raise GitError("base sha is empty or all zeros (new branch or first push)")
    if not head:
        raise GitError("head revision is empty")
    for rev in (base, head):
        probe = subprocess.run(
            ["git", "cat-file", "-e", rev + "^{commit}"],
            cwd=cwd, capture_output=True, text=True,
        )
        if probe.returncode != 0:
            raise GitError("revision %r is not available locally" % rev)
    dots = "..." if range_kind == "pr" else ".."
    proc = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "-z", base + dots + head],
        cwd=cwd, capture_output=True,
    )
    if proc.returncode != 0:
        raise GitError("git diff failed: " + proc.stderr.decode(errors="replace").strip())
    return [p for p in proc.stdout.decode("utf-8", errors="surrogateescape").split("\0") if p]


def compute(args):
    """Return the output dict; never raises."""
    try:
        if args.full:
            return full_outputs("--full requested")
        if args.paths_from:
            paths = [ln for ln in Path(args.paths_from).read_text().splitlines() if ln.strip()]
        else:
            paths = git_changed_paths(args.base, args.head, args.range, args.root)
        if not paths:
            return full_outputs("empty diff; cannot prove nothing changed")
        tokens, sel_reasons, families = select(paths, args.root)
        n = len(paths)
        return build_outputs(
            "selected", tokens, lambda lane: _why(lane & tokens, sel_reasons, families, n)
        )
    except GitError as exc:
        return full_outputs("git unavailable: %s" % exc)
    except Exception as exc:  # noqa: BLE001 - fail-safe: any error selects everything
        return full_outputs("selector error: %s: %s" % (type(exc).__name__, exc))


def summary_markdown(out):
    lines = ["### Deep check selection (%s)" % out["mode"], "", "| Lane | Run | Reason |", "|---|---|---|"]
    for lane, key, rkey in (
        ("Miri cross-arena", "cross_arena", "reason_cross_arena"),
        ("Miri pointer controls (%s)" % out["pointer_mode"], "pointer_controls", "reason_pointer_controls"),
        ("Fuzz corpus replay", "fuzz_any", "reason_fuzz"),
    ):
        reason = out[rkey].replace("|", "\\|")
        lines.append("| %s | %s | %s |" % (lane, out[key], reason))
    targets = [e["target"] for e in json.loads(out["fuzz_matrix"])["include"]]
    lines += ["", "Fuzz targets: %s" % (", ".join(targets) or "none"), ""]
    return "\n".join(lines)


def parse_args(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--full", action="store_true", help="select every deep lane")
    ap.add_argument("--paths-from", help="file with changed paths, one per line")
    ap.add_argument("--base", help="base revision")
    ap.add_argument("--head", help="head revision")
    ap.add_argument("--range", choices=("pr", "push"), default="pr")
    ap.add_argument("--root", default=str(REPO_ROOT), help="repository root (default: this repo)")
    args = ap.parse_args(argv)
    if not (args.full or args.paths_from or args.base is not None or args.head is not None):
        ap.error("one of --full, --paths-from or --base/--head is required")
    return args


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    out = compute(args)
    text = "".join("%s=%s\n" % kv for kv in out.items())
    sys.stdout.write(text)
    target = os.environ.get("GITHUB_OUTPUT")
    if target:
        with open(target, "a", encoding="utf-8") as fh:
            fh.write(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write(summary_markdown(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
