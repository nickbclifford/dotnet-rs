#!/usr/bin/env python3
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "ci_select.py"
spec = importlib.util.spec_from_file_location("ci_select", SCRIPT)
sel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sel)

# The fuzz-corpus-replay matrix include list exactly as ci.yml held it statically
# before selection (commit 384280cc). Do not edit without changing the lane itself.
ORIGINAL_FUZZ_INCLUDE = [
    {"target": "fuzz_managed_ptr_roundtrip", "working-directory": "crates/dotnet-value",
     "corpus": "fuzz/corpus/fuzz_managed_ptr_roundtrip", "fuzz-workspace": "crates/dotnet-value/fuzz"},
    {"target": "fuzz_managed_ptr_offset", "working-directory": "crates/dotnet-value",
     "corpus": "fuzz/corpus/fuzz_managed_ptr_offset", "fuzz-workspace": "crates/dotnet-value/fuzz"},
    {"target": "fuzz_raw_memory_access", "working-directory": "crates/dotnet-value",
     "corpus": "fuzz/corpus/fuzz_raw_memory_access", "fuzz-workspace": "crates/dotnet-value/fuzz"},
    {"target": "fuzz_executor", "working-directory": "crates/dotnet-vm/fuzz",
     "corpus": "corpus/fuzz_executor", "fuzz-workspace": "crates/dotnet-vm/fuzz"},
]


def run_paths(paths, root=REPO):
    """Run the selector CLI on a paths file; return the parsed output dict."""
    with tempfile.TemporaryDirectory() as tmp:
        pf = Path(tmp) / "paths"
        pf.write_text("\n".join(paths) + "\n")
        return run_cli(["--paths-from", str(pf), "--root", str(root)])


def run_cli(args, cwd=None):
    proc = subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, text=True, cwd=cwd,
                          env={k: v for k, v in os.environ.items() if k not in ("GITHUB_OUTPUT", "GITHUB_STEP_SUMMARY")})
    assert proc.returncode == 0, proc.stderr
    return dict(line.split("=", 1) for line in proc.stdout.splitlines())


def lanes(out):
    targets = sorted(e["target"] for e in json.loads(out["fuzz_matrix"])["include"])
    return out["cross_arena"], out["pointer_controls"], targets


EVERYTHING = ("true", "true", sorted(e["target"] for e in ORIGINAL_FUZZ_INCLUDE))
VM_ONLY = ("true", "true", ["fuzz_executor"])
NOTHING = ("false", "false", [])


class SelectionTests(unittest.TestCase):
    def check(self, paths, expected, mode="selected"):
        out = run_paths(paths)
        self.assertEqual(lanes(out), expected, out)
        self.assertEqual(out["mode"], mode)
        self.assertEqual(out["fuzz_any"], "true" if expected[2] else "false")
        self.assertEqual(out["pointer_mode"], "batched")
        return out

    def test_docs_only(self):
        self.check(["docs/CI.md", "README.md", "CONTRIBUTING.md", "LICENSE", "experiments/async-effects/src/lib.rs",
                    "crates/dotnet-vm/fuzz/README.md", "crates/dotnet-vm/src/NOTES.md"], NOTHING)

    def test_value_src_everything(self):
        self.check(["crates/dotnet-value/src/pointer/origin.rs"], EVERYTHING)

    def test_value_closure_crates_everything(self):
        for crate in sorted(sel.VALUE_CLOSURE):
            self.check(["crates/%s/src/lib.rs" % crate], EVERYTHING)

    def test_value_fuzz_dir_everything(self):
        self.check(["crates/dotnet-value/fuzz/corpus/fuzz_raw_memory_access/abc"], EVERYTHING)

    def test_vm_only(self):
        self.check(["crates/dotnet-vm/src/executor.rs"], VM_ONLY)

    def test_vm_fuzz_dir_vm_tier(self):
        self.check(["crates/dotnet-vm/fuzz/corpus/fuzz_executor/abc"], VM_ONLY)

    def test_intrinsics_string_like_vm(self):
        self.check(["crates/dotnet-intrinsics-string/src/lib.rs"], VM_ONLY)

    def test_assemblies_support_sources_are_vm_tier(self):
        self.check(["crates/dotnet-assemblies/src/support/Foo.cs"], VM_ONLY)

    def test_cli_and_benchmarks_nothing(self):
        self.check(["crates/dotnet-cli/src/main.rs", "crates/dotnet-cli/tests/x.rs",
                    "crates/dotnet-benchmarks/benches/a.rs"], NOTHING)

    def test_root_build_config_everything(self):
        for path in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml", "xtask/src/main.rs",
                     "clippy.toml", "rustfmt.toml", ".config/nextest.toml"):
            self.check([path], EVERYTHING)

    def test_ci_yml_and_deep_scripts_everything(self):
        for path in (".github/workflows/ci.yml", "scripts/ci_diagnostics.py", "scripts/pointer_provenance.sh",
                     "scripts/replay_fuzz_corpus.py", "scripts/ci_select.py",
                     "scripts/tests/test_replay_fuzz_corpus.py"):
            self.check([path], EVERYTHING)

    def test_unrelated_script_and_workflow_nothing(self):
        self.check(["scripts/profile_perf.sh", "scripts/tests/test_ci_select.py", ".github/workflows/miri.yml",
                    ".github/workflows/fuzz.yml", ".github/workflows/valgrind.yml", "valgrind.supp"], NOTHING)

    def test_unknown_paths_everything_and_named(self):
        for path in ("brand-new-top/file.txt", "crates/dotnet-newcrate/src/lib.rs", ".github/actions/x/action.yml",
                     "crates/stray.rs"):
            out = self.check(["docs/a.md", path], EVERYTHING)
            self.assertIn(path, out["reason_cross_arena"])

    def test_mixed_docs_and_vm(self):
        self.check(["docs/CI.md", "crates/dotnet-vm/src/a.rs", "scripts/profile_perf.sh"], VM_ONLY)

    def test_mixed_vm_and_value_is_everything(self):
        self.check(["crates/dotnet-vm/src/a.rs", "crates/dotnet-value/src/a.rs"], EVERYTHING)

    def test_reasons_are_single_line_and_name_paths(self):
        out = self.check(["crates/dotnet-vm/src/a.rs"], VM_ONLY)
        self.assertIn("crates/dotnet-vm/src/a.rs", out["reason_cross_arena"])
        self.assertIn("not selected", self.check(["docs/x.md"], NOTHING)["reason_fuzz"])
        for value in out.values():
            self.assertNotIn("\n", value)

    def test_md_pulled_in_by_include_str_is_code(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "crates/dotnet-value/src").mkdir(parents=True)
            (root / "crates/dotnet-vm/src").mkdir(parents=True)
            (root / "crates/dotnet-value/src/lib.rs").write_text('#![doc = include_str!("../README.md")]\n')
            (root / "crates/dotnet-vm/src/lib.rs").write_text(
                'const T: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/GUIDE.md"));\n')
            out = run_paths(["crates/dotnet-value/README.md"], root)
            self.assertEqual(lanes(out), EVERYTHING)
            self.assertIn("code-referenced doc", out["reason_cross_arena"])
            out = run_paths(["crates/dotnet-vm/GUIDE.md"], root)
            self.assertEqual(lanes(out), VM_ONLY)
            out = run_paths(["crates/dotnet-vm/OTHER.md"], root)
            self.assertEqual(lanes(out), NOTHING)

    def test_repo_has_no_unexpected_md_includes(self):
        # Informational pin: if this fails, a .md became code; the selector handles it
        # via md_references(), this just forces a conscious look.
        self.assertEqual(sel.md_references(REPO), {})


class FailSafeTests(unittest.TestCase):
    def assert_full(self, out, needle=None):
        self.assertEqual(out["mode"], "full")
        self.assertEqual(lanes(out), EVERYTHING)
        self.assertEqual(out["fuzz_any"], "true")
        self.assertEqual(out["pointer_mode"], "singleton")
        if needle:
            self.assertIn(needle, out["reason_cross_arena"])

    def test_full_flag(self):
        self.assert_full(run_cli(["--full"]), "--full")

    def test_empty_paths_is_full(self):
        self.assert_full(run_paths([]), "empty diff")

    def test_zero_base_is_full(self):
        self.assert_full(run_cli(["--base", "0" * 40, "--head", "HEAD"], cwd=REPO), "zero")

    def test_bad_base_is_full(self):
        self.assert_full(run_cli(["--base", "deadbeef" * 5, "--head", "HEAD"], cwd=REPO), "not available")

    def test_git_failure_is_full(self):
        with tempfile.TemporaryDirectory() as tmp:  # not a git repository
            self.assert_full(run_cli(["--base", "HEAD~1", "--head", "HEAD", "--root", tmp], cwd=tmp), "git unavailable")

    def test_selector_exception_is_full(self):
        out = run_cli(["--paths-from", "/nonexistent/paths"])
        self.assert_full(out, "selector error")

    def test_real_git_ranges(self):
        with tempfile.TemporaryDirectory() as tmp:
            def git(*a):
                return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *a], cwd=tmp, check=True,
                                      capture_output=True, text=True).stdout.strip()
            git("init", "-q", "-b", "main")
            Path(tmp, "README.md").write_text("a\n")
            git("add", "-A"); git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            git("checkout", "-qb", "topic")
            os.makedirs(Path(tmp, "crates/dotnet-vm/src"))
            Path(tmp, "crates/dotnet-vm/src/a.rs").write_text("x\n")
            git("add", "-A"); git("commit", "-qm", "topic")
            head = git("rev-parse", "HEAD")
            git("checkout", "-q", "main")
            Path(tmp, "docs").mkdir(); Path(tmp, "docs/x.md").write_text("x\n")
            git("add", "-A"); git("commit", "-qm", "main moves")
            main = git("rev-parse", "HEAD")
            args = ["--root", tmp]
            self.assertEqual(lanes(run_cli(["--base", base, "--head", head, *args], cwd=tmp)), VM_ONLY)
            # PR form (three dots) ignores what landed on the base afterwards ...
            self.assertEqual(lanes(run_cli(["--base", main, "--head", head, *args], cwd=tmp)), VM_ONLY)
            # ... the push form (two dots) is a direct diff between the two tips.
            out = run_cli(["--base", main, "--head", head, "--range", "push", *args], cwd=tmp)
            self.assertEqual(lanes(out), VM_ONLY)
            self.assertEqual(lanes(run_cli(["--base", base, "--head", main, *args], cwd=tmp)), NOTHING)
            # identical revisions: empty diff is not proof of safety.
            self.assertEqual(run_cli(["--base", head, "--head", head, *args], cwd=tmp)["mode"], "full")


class OutputTests(unittest.TestCase):
    def test_fuzz_matrix_entries_identical_to_original(self):
        full = json.loads(run_cli(["--full"])["fuzz_matrix"])
        self.assertEqual(full, {"include": ORIGINAL_FUZZ_INCLUDE})
        executor = json.loads(run_paths(["crates/dotnet-vm/src/a.rs"])["fuzz_matrix"])
        self.assertEqual(executor["include"], [ORIGINAL_FUZZ_INCLUDE[3]])
        self.assertEqual(json.loads(run_paths(["docs/a.md"])["fuzz_matrix"]), {"include": []})

    def test_github_output_and_summary_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            out_file, sum_file = Path(tmp, "out"), Path(tmp, "sum")
            env = dict(os.environ, GITHUB_OUTPUT=str(out_file), GITHUB_STEP_SUMMARY=str(sum_file))
            subprocess.run([sys.executable, str(SCRIPT), "--full"], check=True, env=env, capture_output=True)
            parsed = dict(line.split("=", 1) for line in out_file.read_text().splitlines())
            self.assertEqual(parsed["mode"], "full")
            self.assertEqual(parsed["pointer_mode"], "singleton")
            self.assertEqual(set(parsed), {"mode", "cross_arena", "pointer_controls", "pointer_mode",
                                           "fuzz_any", "fuzz_matrix", "reason_cross_arena", "reason_pointer_controls", "reason_fuzz"})
            self.assertIn("Deep check selection (full)", sum_file.read_text())

    def test_ci_yml_fallback_matches_full_selection(self):
        """select-deep's literal fallback outputs must equal what --full produces."""
        text = (REPO / ".github/workflows/ci.yml").read_text()
        match = re.search(r"echo 'fuzz_matrix=(\{.*?\})'", text)
        self.assertIsNotNone(match, "select-deep fallback fuzz_matrix literal not found in ci.yml")
        self.assertEqual(json.loads(match.group(1)), {"include": ORIGINAL_FUZZ_INCLUDE})
        self.assertEqual(match.group(1), run_cli(["--full"])["fuzz_matrix"])

    def test_ci_yml_fallback_pointer_mode_is_singleton(self):
        """The fallback must stay a full sweep: singleton pointer controls, never batched."""
        text = (REPO / ".github/workflows/ci.yml").read_text()
        fallback = re.search(r"- id: fallback\n.*?(?=\n      - uses:)", text, re.S).group(0)
        self.assertIn("echo 'pointer_mode=singleton'", fallback)
        self.assertNotIn("batched", fallback)
        self.assertIn("pointer_mode: ${{ steps.select.outputs.pointer_mode || steps.fallback.outputs.pointer_mode }}", text)
        self.assertEqual(run_cli(["--full"])["pointer_mode"], "singleton")

    def test_pointer_mode_by_selection(self):
        # mode=selected (even when every lane is selected) batches; only mode=full is singleton.
        for paths in (["crates/dotnet-vm/src/lib.rs"], ["docs/CI.md"], ["crates/dotnet-value/src/lib.rs"],
                      ["totally/unknown/path"]):
            out = run_paths(paths)
            self.assertEqual((out["mode"], out["pointer_mode"]), ("selected", "batched"), paths)
        # Empty diff, git failure and --full are full sweeps and therefore singleton.
        self.assertEqual(run_paths([])["pointer_mode"], "singleton")
        self.assertEqual(run_cli(["--full"])["pointer_mode"], "singleton")


# --- Closure drift check -------------------------------------------------

DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def path_deps(manifest, workspace_deps):
    data = tomllib.loads(manifest.read_text())
    tables = [data.get(k, {}) for k in DEP_TABLES]
    for target in data.get("target", {}).values():
        tables.extend(target.get(k, {}) for k in DEP_TABLES)
    found = set()
    for table in tables:
        for name, spec in table.items():
            if not isinstance(spec, dict):
                continue
            if "path" in spec:
                found.add((manifest.parent / spec["path"]).resolve())
            elif spec.get("workspace") and "path" in workspace_deps.get(name, {}):
                found.add((REPO / workspace_deps[name]["path"]).resolve())
    return found


def crate_closure(start):
    workspace_deps = tomllib.loads((REPO / "Cargo.toml").read_text())["workspace"]["dependencies"]
    seen, stack = set(), [start.resolve()]
    while stack:
        crate = stack.pop()
        if crate in seen:
            continue
        seen.add(crate)
        stack.extend(path_deps(crate / "Cargo.toml", workspace_deps))
    return seen


def crate_names(dirs):
    return {d.name for d in dirs if d.parent == (REPO / "crates").resolve()}


class ClosureDriftTests(unittest.TestCase):
    """The hardcoded closure table must equal the closure derived from Cargo.toml."""

    def test_vm_closure(self):
        derived = crate_names(crate_closure(REPO / "crates/dotnet-vm"))
        self.assertEqual(sel.VM_CLOSURE, derived,
                         "VM_CLOSURE drifted; missing=%s stale=%s" % (sorted(derived - sel.VM_CLOSURE),
                                                                      sorted(sel.VM_CLOSURE - derived)))

    def test_value_closure_from_fuzz_workspace(self):
        derived = crate_names(crate_closure(REPO / "crates/dotnet-value/fuzz"))
        self.assertEqual(sel.VALUE_CLOSURE, derived,
                         "VALUE_CLOSURE drifted; missing=%s stale=%s" % (sorted(derived - sel.VALUE_CLOSURE),
                                                                         sorted(sel.VALUE_CLOSURE - derived)))

    def test_vm_fuzz_workspace_closure_is_vm_closure(self):
        derived = crate_names(crate_closure(REPO / "crates/dotnet-vm/fuzz"))
        self.assertEqual(derived, sel.VM_CLOSURE)

    def test_value_closure_is_inside_vm_closure(self):
        self.assertLessEqual(sel.VALUE_CLOSURE, sel.VM_CLOSURE)

    def test_every_workspace_crate_is_classified(self):
        members = tomllib.loads((REPO / "Cargo.toml").read_text())["workspace"]["members"]
        crates = {Path(m).name for m in members if m.startswith("crates/")}
        self.assertEqual(crates - sel.VM_CLOSURE, set(sel.NON_DEEP_CRATES),
                         "a workspace crate is neither in the VM closure nor listed as non-deep")
        on_disk = {p.name for p in (REPO / "crates").iterdir() if p.is_dir()}
        self.assertEqual(on_disk, crates | set(), "crates/ directory and workspace members differ")

    def test_non_deep_crates_are_not_dependencies_of_closure(self):
        self.assertFalse(set(sel.NON_DEEP_CRATES) & sel.VM_CLOSURE)


if __name__ == "__main__":
    unittest.main()
