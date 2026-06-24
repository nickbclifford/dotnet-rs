# REVIEW — NuGet Host Runner (Option A Milestones 1–2 + EF Spike)

> Kickoff date: 2026-06-23. Branch: `supervised/nuget-host-runner`.
> Planner: claude-sonnet-4-6.

---

## Executive Summary

**Two-stage plan.** Stage 1 is a read-only empirical probe: assemble the EF InMemory package closure
into a flat directory and run it under the existing `--assemblies` loader, recording every gap. The
deliverable is a prioritized gap backlog and a yes/no answer on "does EF InMemory's core engine run
end-to-end on `dotnet-rs`?" Stage 2 builds a real host path (Option A, milestones 1–2) that reads the
machine-readable metadata `dotnet` already produces — `*.runtimeconfig.json` for framework selection
with proper roll-forward, `*.deps.json` for managed and native asset probing — generalizing the flat-
directory loader while keeping `--assemblies` fully working.

**Hard constraints (must hold after every step):**
- `--assemblies <FOLDER>` works exactly as it does today. The integration-test harness depends on it.
- NativeAOT-published inputs fail loudly with an explicit "unsupported" message. Single-file bundles fail
  loudly. ReadyToRun images are fine (they carry IL).
- The `IsDynamicCodeSupported = false` switch in `crates/dotnet-vm/src/state.rs:447–455` must not
  be disturbed; the entire EF path depends on routing `Expression.Compile` through the BCL interpreter.

**Severity summary:**

| Finding | Category | Severity |
|---|---|---|
| F-SPIKE-001 | SPIKE | Blocking (Stage 1 deliverable) |
| F-HOST-001 | HOST | High (Milestone 1 core) |
| F-HOST-002 | HOST | High (Milestone 1 algorithm) |
| F-HOST-003 | HOST | High (Milestone 2 core) |
| F-LOAD-001 | LOAD | High (single-root limitation) |
| F-LOAD-002 | LOAD | Medium (probing-paths extension) |
| F-LOAD-003 | LOAD | Medium (new constructor) |
| F-CLI-001 | CLI | Medium (additive, preserves --assemblies) |
| F-COMPAT-001 | COMPAT | Medium (fail-loud, not a crash) |
| F-TEST-001 | TEST | Medium (3-rung ladder) |
| F-DOC-001 | DOC | Low (roadmap update) |

---

## Methodology

**Files read:** `crates/dotnet-assemblies/src/loader.rs` (full), `crates/dotnet-assemblies/src/resolution.rs` (full), `crates/dotnet-assemblies/src/lib.rs`, `crates/dotnet-assemblies/Cargo.toml`, `crates/dotnet-cli/src/lib.rs`, `crates/dotnet-cli/Cargo.toml`, `crates/dotnet-cli/tests/integration_tests_impl/harness.rs`, `check.sh`, `scripts/diff_run.sh`, `docs/USERLAND_TESTING_ROADMAP.md` (full), `crates/dotnet-vm/src/state.rs` (lines 430–460), `Cargo.toml` (workspace).

**Commands run (all read-only or in /tmp):**
- `dotnet --version && dotnet --list-runtimes` → SDK 10.0.109; runtimes `8.0.28` and `10.0.9`.
- `ls ~/.nuget/packages | wc -l` → 431 packages.
- `ls ~/.nuget/packages | grep entityframeworkcore` → initially absent; after probe restore → present.
- Built `crates/dotnet-cli/tests/fixtures/expressions/expression_compile_42.cs` via `SingleFile.csproj` → confirmed `SingleFile.runtimeconfig.json` + `SingleFile.deps.json` output.
- Created `/tmp/nuget-probe/` with `Newtonsoft.Json 13.0.3` → examined `App.deps.json` and `App.runtimeconfig.json`.
- Created `/tmp/ef-probe/` with `Microsoft.EntityFrameworkCore.InMemory 9.0.*` → restored packages, built, confirmed `dotnet /tmp/ef-probe-out/EfApp.dll` exits 42 under stock dotnet.
- Confirmed EF packages now in `~/.nuget/packages`: `microsoft.entityframeworkcore`, `microsoft.entityframeworkcore.abstractions`, `microsoft.entityframeworkcore.analyzers`, `microsoft.entityframeworkcore.inmemory`, plus the full `Microsoft.Extensions.*` closure (11 DLLs total in build output).

All claims in findings below are grounded against the live code at the cited file:line locations.

---

## Findings

<a id="F-SPIKE-001"></a>
#### F-SPIKE-001 — EF InMemory smoke: packages restored, flat-dir assembly needed

**Current state.** At kickoff, `microsoft.entityframeworkcore*` were absent from `~/.nuget/packages`.
During the planning probe they were restored. As of this writing they exist at:
```
~/.nuget/packages/microsoft.entityframeworkcore/9.0.17/
~/.nuget/packages/microsoft.entityframeworkcore.inmemory/9.0.17/
~/.nuget/packages/microsoft.extensions.{caching,dependencyinjection,logging,options,primitives}.*/9.0.17/
```
The EF app (`/tmp/ef-probe-out/EfApp.dll`) was built and confirmed to exit 42 under stock `dotnet`.
The DLL closure in the build output flat dir:
```
EfApp.dll
Microsoft.EntityFrameworkCore.dll           (net8.0 asset)
Microsoft.EntityFrameworkCore.Abstractions.dll
Microsoft.EntityFrameworkCore.InMemory.dll
Microsoft.Extensions.Caching.Abstractions.dll
Microsoft.Extensions.Caching.Memory.dll
Microsoft.Extensions.DependencyInjection.dll
Microsoft.Extensions.DependencyInjection.Abstractions.dll
Microsoft.Extensions.Logging.dll
Microsoft.Extensions.Logging.Abstractions.dll
Microsoft.Extensions.Options.dll
Microsoft.Extensions.Primitives.dll
```

**What the spike step does.** Create a flat directory combining the framework DLLs (`/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.9/`) and the EF closure DLLs (above). Run the EF app under `dotnet-rs -a <flat-dir> /tmp/ef-probe-out/EfApp.dll`. **Do not fix any gaps** — record every failure (unimplemented intrinsic, missing opcode, reflection gap, type-resolution failure) with its exception/error message. Produce a prioritized backlog (P1 = engine blocker, P2 = reflection/attribute gap, P3 = minor correctness issue). Write the backlog to `docs/EF_GAP_BACKLOG.md`.

**Risk.** The EF app may hang, crash, or surface many layers of gaps before producing output. Timeout the run at 60 seconds. If execution makes no progress past DI container construction, note that as the blocker.

**Effort.** High (exploratory; requires creating flat dir, running under dotnet-rs with tracing, interpreting any output or panic, and writing the backlog document).

**Note.** `/tmp/` is ephemeral. If the probe `/tmp/ef-probe-out/` is missing, rebuild with:
```bash
mkdir -p /tmp/ef-probe && # (recreate EfApp.csproj + Program.cs per this file's probe section)
cd /tmp/ef-probe && dotnet build EfApp.csproj -o /tmp/ef-probe-out -p:BaseIntermediateOutputPath=/tmp/ef-probe-out/obj/ -v:q --nologo
```

---

<a id="F-HOST-001"></a>
#### F-HOST-001 — `runtimeconfig.json` structure and serde parsing

**Current state.** The runtime today is located by `find_dotnet_app_path()` in
`crates/dotnet-assemblies/src/resolution.rs:902–928`, which calls `find_latest_runtime_in_base()`
(`resolution.rs:871–899`). This scans a well-known base path for the latest version subdirectory —
a single candidate, no policy. There is no reading of any `*.runtimeconfig.json`.

**Observed `*.runtimeconfig.json` structure** (from `SingleFile` fixture build and `Newtonsoft.Json` probe):
```json
{
  "runtimeOptions": {
    "tfm": "net10.0",
    "framework": {
      "name": "Microsoft.NETCore.App",
      "version": "10.0.0"
    },
    "configProperties": {
      "System.Runtime.Serialization.EnableUnsafeBinaryFormatterSerialization": false
    }
  }
}
```
The `rollForward` key is absent when using the default policy. Older configs use `rollForwardOnNoCandidateFx` (legacy; can be ignored for new code; tolerated by treating absent as the default `Minor` policy).

**Proposed change.** Add `crates/dotnet-assemblies/src/host.rs` with:
- `RuntimeConfig`, `RuntimeOptions`, `FrameworkRef`, `RollForwardPolicy` types with `serde::Deserialize`.
- `parse_runtimeconfig(path: &Path) -> Result<RuntimeConfig, HostError>` using `serde_json`.
- `HostError` error type (via `thiserror`).

Add `serde = { version = "1", features = ["derive"] }` and `serde_json = "1"` to both workspace
`Cargo.toml` `[workspace.dependencies]` and `crates/dotnet-assemblies/Cargo.toml` `[dependencies]`.

Unit test: parse the fixture's `SingleFile.runtimeconfig.json` (built in `/tmp/fixture-probe/`) and
assert the parsed fields match the known values.

**Risk.** Low — additive, no existing code changes.

**Effort.** Default.

---

<a id="F-HOST-002"></a>
#### F-HOST-002 — Roll-forward version selection algorithm

**Current state.** `find_latest_runtime_in_base(resolution.rs:871–899)` sorts all version-named
subdirectories and returns the highest. This corresponds to `LatestMajor` — always the absolute
newest. The real default policy is `Minor`, which is more conservative.

**Installed runtimes (verified):** `Microsoft.NETCore.App 8.0.28` and `10.0.9` under
`/usr/share/dotnet/shared/Microsoft.NETCore.App/`. The fixture `SingleFile.runtimeconfig.json`
requests `10.0.0`. Under `Minor` policy: request `10.0.0`, installed `10.0.9` → select `10.0.9`
(same major, same minor, highest patch). Under the old "latest" logic, `10.0.9` would also be
selected (it's the highest), so for the currently installed runtimes both algorithms agree. But
if `11.0.x` were installed, the old code would select it incorrectly.

**Algorithm for the six policies** (reference: [.NET version selection docs](https://learn.microsoft.com/dotnet/core/versions/selection)):

| Policy | Selection rule |
|---|---|
| `Disable` | Exact match on major.minor.patch; fail if none |
| `LatestPatch` | Same major.minor as requested; highest patch |
| `Minor` (default) | Same major; lowest minor ≥ requested.minor; then highest patch at that minor |
| `LatestMinor` | Same major; highest minor; then highest patch |
| `Major` | Lowest major ≥ requested.major; then `Minor` logic within that major |
| `LatestMajor` | Highest major; then `LatestMinor` logic |

**Proposed change.** In `crates/dotnet-assemblies/src/host.rs`, add:
```rust
pub fn select_framework_version(
    base_dir: &Path,           // e.g., /usr/share/dotnet/shared/Microsoft.NETCore.App
    requested: &FrameworkRef,  // name + version string from runtimeconfig
    policy: RollForwardPolicy,
) -> Option<PathBuf>
```
This function scans `base_dir` for numeric subdirectories (same approach as current
`find_latest_runtime_in_base`), parses them as `(major, minor, patch)` triples, applies the policy,
and returns the selected directory.

Add `pub fn resolve_framework_from_runtimeconfig(config: &RuntimeConfig, override_base: Option<&Path>) -> Option<PathBuf>` that combines parsing + selection and handles the `DOTNET_ROOT` env var for overriding the base.

Unit tests: given mock directories `8.0.28` and `10.0.9`, assert each policy selects the correct one for a request of `10.0.0`. Boundary cases: request `10.0.0`, `Disable` policy, no exact match → `None`.

**Risk.** Medium — algorithm must be correct; wrong selection could cause type-load failures. Unit tests are essential before wiring up.

**Effort.** High.

---

<a id="F-HOST-003"></a>
#### F-HOST-003 — `deps.json` structure and managed/native probing list derivation

**Current state.** No `deps.json` reading exists anywhere in the codebase. The test harness builds
fixtures via `SingleFile.csproj` which does emit a `deps.json`, but it is never read.

**Observed `*.deps.json` structure** (from Newtonsoft.Json probe, abridged):
```json
{
  "runtimeTarget": { "name": ".NETCoreApp,Version=v10.0" },
  "targets": {
    ".NETCoreApp,Version=v10.0": {
      "App/1.0.0": {
        "dependencies": { "Newtonsoft.Json": "13.0.3" },
        "runtime": { "App.dll": {} }
      },
      "Newtonsoft.Json/13.0.3": {
        "runtime": {
          "lib/net6.0/Newtonsoft.Json.dll": {
            "assemblyVersion": "13.0.0.0",
            "fileVersion": "13.0.3.27908"
          }
        }
      }
    }
  },
  "libraries": {
    "App/1.0.0": { "type": "project", "serviceable": false, "sha512": "" },
    "Newtonsoft.Json/13.0.3": {
      "type": "package",
      "serviceable": true,
      "path": "newtonsoft.json/13.0.3"
    }
  }
}
```

**Managed assembly path derivation:**
```
For each (pkg_key, target_lib) in targets[runtimeTarget.name]:
    lib_info = libraries[pkg_key]
    if lib_info.type == "package" && lib_info.path is set:
        pkg_dir = nuget_global_packages + "/" + lib_info.path
        for each asset_rel_path in target_lib.runtime.keys():
            full_path = pkg_dir + "/" + asset_rel_path
            managed_assemblies.push(full_path)
    if lib_info.type == "project" (the app itself):
        app_dir (where the entry DLL lives) covers this
```

**Native search directory derivation** (for future P/Invoke wiring — derived but not wired this pass):
```
For each (pkg_key, target_lib) in targets[runtimeTarget.name]:
    if target_lib.native is non-empty:
        lib_info = libraries[pkg_key]
        pkg_dir = nuget_global_packages + "/" + lib_info.path
        for each native_rel_path in target_lib.native.keys():
            native_dir = pkg_dir + "/" + parent_of(native_rel_path)
            native_search_dirs.push(native_dir)
```

**RID asset selection.** Modern .NET uses a portable-RID fallback list (no RID graph). For `linux-x64` the fallback is: `linux-x64 → linux → unix-x64 → unix → any`. Assets are selected by checking the `rid` field in `runtimeTarget`-level asset groups. For most managed packages there are no RID-specific assets (just `runtime.*` entries), so RID handling is not needed for managed DLLs — only for native assets.

**NuGet global packages folder.** Resolved via: `$NUGET_PACKAGES` env var → `~/.nuget/packages` (Linux/macOS) → `%USERPROFILE%\.nuget\packages` (Windows).

**Proposed change.** In `crates/dotnet-assemblies/src/host.rs`, add:
- `DepsJson`, `TargetLibrary`, `AssemblyAssetInfo`, `LibraryInfo` types with `serde::Deserialize`.
- `parse_deps_json(path: &Path) -> Result<DepsJson, HostError>`.
- `derive_managed_probing_paths(deps: &DepsJson, nuget_global: &Path) -> Vec<(String, PathBuf)>` — returns `(assembly_stem, full_path)` pairs.
- `derive_native_search_dirs(deps: &DepsJson, nuget_global: &Path) -> Vec<PathBuf>`.
- `nuget_global_packages_dir() -> PathBuf` — env var / platform default.

**Risk.** Low — additive. The native search dirs are derived but not wired (downstream milestone 4).

**Effort.** Default.

---

<a id="F-LOAD-001"></a>
#### F-LOAD-001 — Current `AssemblyLoader` is single-root only

**Current state.** `AssemblyLoader::new(assembly_root: String)` (`loader.rs:147–159`) scans exactly
one directory for `*.dll` and registers each stem as `None` (lazy). `load_and_register`
(`loader.rs:286–320`) builds the path as `PathBuf::from(&self.assembly_root).push("{name}.dll")` — the
root is hardcoded to a single string field.

This means the loader cannot express "look in the framework dir AND in the NuGet package paths." All
DLLs for a run must be in one flat directory (the `--assemblies` mode today manually combines them).

**Root cause.** `external: RwLock<HashMap<String, Option<ResolutionS>>>` stores `None` for lazy
assemblies with the implicit contract that the path is always `assembly_root/{name}.dll`. With multiple
roots, `None` is ambiguous (which root?).

**Impact.** Cannot load NuGet packages from their canonical `~/.nuget/packages/<pkg>/<ver>/lib/<tfm>/`
locations. Cannot combine framework DLLs + app DLLs + NuGet DLLs across directories.

**Proposed change.** Add two fields to `AssemblyLoader`:
1. `probing_paths: DashMap<String, PathBuf>` — an explicit name→path registry for assemblies whose
   path is known exactly (NuGet assets from deps.json, assemblies in secondary scan roots).
2. Keep `external: RwLock<HashMap<String, Option<ResolutionS>>>` and `assembly_root: String`
   unchanged — existing `--assemblies` callers are unaffected.

Update `load_and_register` (`loader.rs:286`) to check `probing_paths` first before falling back to
the `assembly_root` path construction. This is a backward-compatible extension: if `probing_paths` is
empty (all existing callers), behavior is identical to today.

**Risk.** Medium — the `load_and_register` / `get_assembly_with_version` chain is performance-critical.
The `DashMap::get` on `probing_paths` is an O(1) lock-free read and adds negligible overhead.
Existing tests verify all currently-passing fixtures remain green.

**Effort.** High (because of the care needed to keep the lock/cache semantics correct and the extensive
test suite run that must follow).

---

<a id="F-LOAD-002"></a>
#### F-LOAD-002 — Secondary scan root support for flat app directories

**Current state.** The loader only scans `assembly_root`. An app built by `dotnet build` produces a
flat output directory containing the app DLL + copies of all NuGet DLLs. To load such an app, the step
agent needs to scan the app's output directory in addition to the framework directory.

**Proposed change.** Add `add_scan_root(root: &Path) -> Result<(), AssemblyLoadError>` to
`AssemblyLoader` that:
1. Scans `root` for `*.dll` files.
2. For each stem not already in `external` (as `Some(_)` — already loaded): inserts `None` into
   `external` and the full path into `probing_paths` (so `load_and_register` can find it).
3. Returns `Ok(())`.

This function is used by `new_from_host` (F-LOAD-003) to register the app's output directory.

**Note.** Primary root (`assembly_root`) wins on name conflict because it was scanned first into
`external`. Secondary root entries via `add_scan_root` go into `probing_paths` only if the name is
absent from `external`.

**Risk.** Low — additive; tested by the test ladder (F-TEST-001).

**Effort.** Default (builds on F-LOAD-001 refactor).

---

<a id="F-LOAD-003"></a>
#### F-LOAD-003 — New `AssemblyLoader::new_from_host` constructor

**Current state.** No host-aware constructor exists. The only entry points are `new(assembly_root)`,
`new_bare(assembly_root)`, and `new_internal`.

**Proposed change.** Add to `AssemblyLoader`:
```rust
pub fn new_from_host(
    entrypoint: &Path,
    nuget_global: Option<&Path>,  // override; None → use nuget_global_packages_dir()
) -> Result<Self, HostError>
```

Implementation:
1. Derive `runtimeconfig_path = entrypoint.with_extension("").runtimeconfig.json` or by replacing
   the entrypoint filename.
2. `config = parse_runtimeconfig(runtimeconfig_path)?`
3. `framework_dir = resolve_framework_from_runtimeconfig(&config, None)?`
4. `loader = AssemblyLoader::new(framework_dir.to_str().unwrap())?`  ← primary root = framework
5. `loader.add_scan_root(entrypoint.parent())?`  ← register app output dir
6. Optionally, parse `deps.json` for explicit NuGet paths (for packages not copied to the flat output
   dir — uncommon for framework-dependent builds but correct in general):
   `deps = parse_deps_json(deps_json_path)?`
   `for (name, path) in derive_managed_probing_paths(&deps, nuget_global_dir): loader.register_probing_path(name, path)?`
7. Return `Ok(loader)`.

**Note.** For framework-dependent `dotnet build` output, the build copies NuGet DLLs to the flat
output dir, so step 5 (`add_scan_root`) already covers them. Step 6 (explicit NuGet paths from
deps.json) is a belt-and-suspenders correctness measure and future-proofs for self-contained deploys
where copies aren't present.

**Risk.** Low — additive. Existing tests call `AssemblyLoader::new()` and are unaffected.

**Effort.** Default (depends on F-HOST-001, F-HOST-002, F-HOST-003, F-LOAD-001, F-LOAD-002).

---

<a id="F-CLI-001"></a>
#### F-CLI-001 — Make `--assemblies` optional; add auto-detect host mode

**Current state.** `Args { assemblies: String }` (`crates/dotnet-cli/src/lib.rs:24–29`) has
`--assemblies` as a required argument. `run_cli()` (`lib.rs:54`) always calls
`AssemblyLoader::new(args.assemblies)`. The `diff_run.sh` script always passes `-a
<shared_framework_dir>`.

**Proposed change.** Make `assemblies: Option<String>` in `Args`. In `run_cli()`:
- If `assemblies` is `Some(dir)`: existing path (`AssemblyLoader::new(dir)`). Unchanged behavior.
- If `assemblies` is `None`: look for `<entrypoint_stem>.runtimeconfig.json` next to the entry DLL.
  If found: use `AssemblyLoader::new_from_host(entrypoint_path, None)`.
  If not found: emit `"error: no --assemblies flag and no <name>.runtimeconfig.json found next to <dll>"` and exit 1.

The `--assemblies` / `-a` short flag is kept. The test harness always passes `-a` → no behavior
change. `diff_run.sh` always passes `-a` → no behavior change.

**Risk.** Low — the `assemblies: Option<String>` change requires updating all callers. The harness
`run_cli` method calls the binary as a subprocess via the `--assemblies` flag; that path is unaffected.
Any in-crate code that calls `Args::parse()` and unconditionally reads `args.assemblies` will fail to
compile, making breakages visible at compile time.

**Effort.** Default.

---

<a id="F-COMPAT-001"></a>
#### F-COMPAT-001 — Fail loudly on NativeAOT and single-file bundle inputs

**Current state.** If a NativeAOT executable or a single-file bundle is passed as the entry DLL,
`dotnetdll`'s `Resolution::parse` will either panic, return a parse error, or succeed with a
nonsensical module. The user sees a confusing error or a crash.

**NativeAOT detection.** NativeAOT publish produces a native ELF/PE with no managed CLI header.
`dotnetdll` will return an error during parse. After the parse attempt, if the error variant indicates
"not a PE" or "no CLI header", emit: `"error: the input appears to be a NativeAOT native executable,
which is not supported (no IL is present). Use a framework-dependent or ReadyToRun build instead."`

**Single-file bundle detection.** Single-file bundles append a bundle manifest after the PE. The
bundle is identified by reading the last N bytes and looking for the bundle signature magic
(`b"BN\x4f\xb1\x47\x00\xb1\xc1\xad\xb3\xc2\xe9\x56\x79\x00\xb7"` — 16 bytes). In practice, a
simpler heuristic: attempt to open the file, read the last 16 bytes, check for the magic. If found:
`"error: the input is a single-file bundle, which is not yet supported. Publish without
/p:PublishSingleFile=true instead."`

**Proposed change.** Add `probe_entry_kind(path: &Path) -> EntryKind` (enum:
`Managed`, `NativeAot`, `SingleFileBundle`) in `crates/dotnet-assemblies/src/host.rs` or a new
`crates/dotnet-assemblies/src/probe.rs`. Call it in `run_cli()` (CLI) and `new_from_host()` (host
constructor) before attempting `load_resolution_from_file`. Emit the appropriate error and return a
non-zero exit code.

**Risk.** Low — these inputs were previously undefined behavior (crash/confuse). Making them explicit
errors is strictly better.

**Effort.** Default.

---

<a id="F-TEST-001"></a>
#### F-TEST-001 — Test ladder: three independently verifiable rungs

**Rung 1 — Fixtures via host path.** After F-CLI-001 lands, build the `expression_compile_42` fixture
and run it as `dotnet-rs /path/to/SingleFile.dll` (without `-a`). The `.runtimeconfig.json` and
`.deps.json` next to the DLL will be found automatically. Verify the exit code matches the fixture's
expected code (42). Then run the full fixture suite under the new mode and confirm no regressions.

**Rung 2 — Newtonsoft.Json via host path.** Build the Newtonsoft.Json probe app (`/tmp/nuget-probe/`
or rebuild from scratch with `App.csproj` + `Program.cs`). Run `dotnet-rs /tmp/nuget-probe-out/App.dll`.
Compare against `dotnet /tmp/nuget-probe-out/App.dll` — both should output `{"name":"test","value":42}`
and exit 42. Verify via `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`.

**Rung 3 — EF InMemory via host path.** After the loader supports multi-root probing, run
`dotnet-rs /tmp/ef-probe-out/EfApp.dll`. Compare against `dotnet EfApp.dll`. This is the acid test
for the full host path end-to-end. The gap backlog from F-SPIKE-001 will have named any blockers.

**Proposed verification for each rung.**
- Rung 1: `cargo nextest run --no-default-features -p dotnet-cli [filter expression_compile]` + manual `dotnet-rs` invocation.
- Rung 2: `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`.
- Rung 3: `bash scripts/diff_run.sh /tmp/ef-probe/EfApp.csproj` (or directly via the `.dll`).

**Risk.** Rung 3 depends on the EF gap backlog being empty or manageable. If there are P1 gaps, rung 3
will fail and the gaps become future work (to be recorded but NOT fixed in this pass).

**Effort.** Default per rung (depends on all prior phases).

---

<a id="F-DOC-001"></a>
#### F-DOC-001 — Update `docs/USERLAND_TESTING_ROADMAP.md`

**Current state.** §10 probe 5 (EF InMemory smoke) is listed as pending. Option A milestones 1–2 are
listed as future work.

**Proposed change.** After Phases 1–5 complete:
- Mark §10 probe 5 with its result: if EF ran end-to-end, mark ✅ with date; if not, mark ❌ and cite
  the gap backlog at `docs/EF_GAP_BACKLOG.md`.
- Mark Option A milestone 1 (runtimeconfig + roll-forward) ✅ with date.
- Mark Option A milestone 2 (deps.json + multi-root probing) ✅ with date.
- Add a one-paragraph summary of the gap backlog from the EF spike under §5 or §10.

**Risk.** None — doc-only change.

**Effort.** Default.

---

## Phase Plan

### Phase 1: EF InMemory Spike (empirical, read-only)

| Step | Description | refs |
|---|---|---|
| 1.1 | Assemble EF flat directory and run spike; record gap backlog | [F-SPIKE-001](#F-SPIKE-001) |

The spike MUST come first: it validates whether the expression-interpreter + reflection substrate is
sufficient for EF's core engine before investing in loader infrastructure. Its output is `docs/EF_GAP_BACKLOG.md`; it does not fix any gaps.

### Phase 2: `runtimeconfig.json` Parser + Roll-forward (Option A Milestone 1)

| Step | Description | refs |
|---|---|---|
| 2.1 | Add `RuntimeConfig` serde types + `parse_runtimeconfig()` in `host.rs`; add serde deps | [F-HOST-001](#F-HOST-001) |
| 2.2 | Implement roll-forward version selection (all 6 policies); unit tests | [F-HOST-002](#F-HOST-002) |
| 2.3 | Add `resolve_framework_from_runtimeconfig()` combining parse + selection | [F-HOST-001](#F-HOST-001), [F-HOST-002](#F-HOST-002) |

### Phase 3: `deps.json` Parser + Multi-root Probing (Option A Milestone 2)

| Step | Description | refs |
|---|---|---|
| 3.1 | Add `DepsJson` serde types + `parse_deps_json()` + `derive_managed_probing_paths()` + `derive_native_search_dirs()` in `host.rs` | [F-HOST-003](#F-HOST-003) |
| 3.2 | Add `probing_paths: DashMap<String, PathBuf>` to `AssemblyLoader`; update `load_and_register` to check it first; add `add_scan_root()` and `register_probing_path()` methods | [F-LOAD-001](#F-LOAD-001), [F-LOAD-002](#F-LOAD-002) |
| 3.3 | Add `AssemblyLoader::new_from_host(entrypoint, nuget_global)` constructor | [F-LOAD-003](#F-LOAD-003) |

### Phase 4: CLI Host Mode

| Step | Description | refs |
|---|---|---|
| 4.1 | Make `--assemblies` optional in `Args`; add auto-detect host mode in `run_cli()` | [F-CLI-001](#F-CLI-001) |

### Phase 5: Test Ladder

| Step | Description | refs |
|---|---|---|
| 5.1 | Rung 1: run existing fixture via host path (no `-a`); verify parity | [F-TEST-001](#F-TEST-001) |
| 5.2 | Rung 2: run Newtonsoft.Json app via host path; verify via diff_run.sh | [F-TEST-001](#F-TEST-001) |
| 5.3 | Rung 3: attempt EF InMemory via host path; record result vs gap backlog | [F-TEST-001](#F-TEST-001), [F-SPIKE-001](#F-SPIKE-001) |

### Phase 6: Fail-loud Compat Handling

| Step | Description | refs |
|---|---|---|
| 6.1 | Detect NativeAOT and single-file bundles; emit clear error messages | [F-COMPAT-001](#F-COMPAT-001) |

### Phase 7: Roadmap Documentation Update

| Step | Description | refs |
|---|---|---|
| 7.1 | Update `docs/USERLAND_TESTING_ROADMAP.md` with spike results and milestone completion status | [F-DOC-001](#F-DOC-001) |

---

## Verification Scope Table

| Kind of change | Narrowest correct verification |
|---|---|
| New types/parse in `dotnet-assemblies` | `cargo clippy --no-default-features -p dotnet-assemblies -- -D warnings && cargo nextest run --no-default-features -p dotnet-assemblies` |
| Loader refactor (`probing_paths`, `add_scan_root`) | Above + `DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run --no-default-features -p dotnet-cli` (all fixture tests must pass) |
| CLI changes (`run_cli`, `Args`) | `cargo nextest run --no-default-features -p dotnet-cli` + manual `cargo run --bin dotnet-rs --no-default-features -- <args> <dll>` smoke |
| EF spike (exploratory, no code changes) | Manual: `cargo run --bin dotnet-rs --no-default-features -- -a <flat-dir> /tmp/ef-probe-out/EfApp.dll` |
| Host path (new_from_host) smoke | `cargo run --bin dotnet-rs --no-default-features -- /path/to/fixture.dll` (no -a) |
| Newtonsoft.Json probe | `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` |
| EF host-path rung | `bash scripts/diff_run.sh /tmp/ef-probe/EfApp.csproj` |
| Any pre-merge confidence check | `bash check.sh` (after `cargo run -p xtask -- fixtures build` and `export DOTNET_USE_PREBUILT_FIXTURES=1`) |
| Always | `cargo fmt --all -- --check` before completing any step |
