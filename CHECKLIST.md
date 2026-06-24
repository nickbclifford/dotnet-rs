# CHECKLIST — NuGet Host Runner (Option A Milestones 1–2 + EF Spike)

> Branch: `supervised/nuget-host-runner`. Tick each box as you complete the step.

## Phase 1: EF InMemory Spike (empirical, read-only)

- [x] 1.1 Assemble EF flat directory (framework DLLs + EF closure DLLs), run `dotnet-rs -a <flat-dir> /tmp/ef-probe-out/EfApp.dll`, capture every failure, write prioritized gap backlog to `docs/EF_GAP_BACKLOG.md` — does NOT fix any gaps [effort: high] — refs REVIEW.md#F-SPIKE-001

## Phase 2: `runtimeconfig.json` Parser + Roll-forward (Option A Milestone 1)

- [x] 2.1 Add `crates/dotnet-assemblies/src/host.rs` with `RuntimeConfig` / `RuntimeOptions` / `FrameworkRef` / `RollForwardPolicy` serde types and `parse_runtimeconfig(path) -> Result<RuntimeConfig, HostError>`; add `serde` + `serde_json` to workspace `Cargo.toml` and `crates/dotnet-assemblies/Cargo.toml`; unit test parsing a real fixture runtimeconfig [effort: default] — refs REVIEW.md#F-HOST-001
- [x] 2.2 Implement `select_framework_version(base_dir, requested, policy) -> Option<PathBuf>` in `host.rs` covering all six roll-forward policies (`Disable`, `LatestPatch`, `Minor`, `LatestMinor`, `Major`, `LatestMajor`); unit tests against installed runtimes 8.0.28 and 10.0.9 for a 10.0.0 request [effort: high] — refs REVIEW.md#F-HOST-002
- [x] 2.3 Add `resolve_framework_from_runtimeconfig(config: &RuntimeConfig, override_base: Option<&Path>) -> Option<PathBuf>` wiring together parse + selection; respect `DOTNET_ROOT` env var; unit test end-to-end from a fixture runtimeconfig path to a resolved directory [effort: default] — refs REVIEW.md#F-HOST-001, REVIEW.md#F-HOST-002

## Phase 3: `deps.json` Parser + Multi-root Probing (Option A Milestone 2)

- [x] 3.1 Add `DepsJson` / `TargetLibrary` / `LibraryInfo` serde types and `parse_deps_json(path) -> Result<DepsJson, HostError>` in `host.rs`; add `derive_managed_probing_paths(deps, nuget_global) -> Vec<(String, PathBuf)>` and `derive_native_search_dirs(deps, nuget_global) -> Vec<PathBuf>`; add `nuget_global_packages_dir() -> PathBuf`; unit test with both the fixture deps.json (no NuGet) and the Newtonsoft.Json deps.json (one NuGet package) [effort: default] — refs REVIEW.md#F-HOST-003
- [x] 3.2 Add `probing_paths: DashMap<String, PathBuf>` field to `AssemblyLoader`; update `load_and_register` to check `probing_paths` before constructing the `assembly_root`-based path; add `add_scan_root(root: &Path) -> Result<(), AssemblyLoadError>` (scans flat dir, inserts into both `external` and `probing_paths` for names not already loaded); add `register_probing_path(name: &str, path: PathBuf)` helper; confirm all existing fixture tests pass with no behavior change [effort: high] — refs REVIEW.md#F-LOAD-001, REVIEW.md#F-LOAD-002
- [x] 3.3 Add `AssemblyLoader::new_from_host(entrypoint: &Path, nuget_global: Option<&Path>) -> Result<Self, HostError>`: parse runtimeconfig → roll-forward → `new(framework_dir)` → `add_scan_root(app_dir)` → parse deps.json → `register_probing_path` for each NuGet asset; expose `host.rs` types from `crates/dotnet-assemblies/src/lib.rs` [effort: default] — refs REVIEW.md#F-LOAD-003
- [x] 3.4 Wire host-derived native search directories/runtime native assets into `dotnet-pinvoke::NativeLibraries` / VM state so framework native libraries such as `libSystem.Native` resolve without manually copying `.so` files into the assembly root [effort: high] — refs REVIEW.md#F-HOST-003

## Phase 4: CLI Host Mode

- [x] 4.1 Change `Args.assemblies` from `String` to `Option<String>` in `crates/dotnet-cli/src/lib.rs`; in `run_cli()`: if `Some(dir)` → existing `AssemblyLoader::new(dir)` path (unchanged); if `None` → `AssemblyLoader::new_from_host(entrypoint_path, None)` with a clear error if no runtimeconfig found; update `--assemblies` / `-a` clap annotation to `required = false` [effort: default] — refs REVIEW.md#F-CLI-001

## Phase 5: Test Ladder

- [x] 5.1 Rung 1: build `expression_compile_42` fixture (or reuse prebuilt), run `dotnet-rs /path/to/SingleFile.dll` (no `-a` flag), confirm exit code 42; run full fixture suite via `DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run --no-default-features -p dotnet-cli` and confirm no regressions [effort: default] — refs REVIEW.md#F-TEST-001
- [ ] 5.2 Rung 2: build the Newtonsoft.Json probe (`/tmp/nuget-probe/App.csproj` or rebuild), run `dotnet-rs /tmp/nuget-probe-out/App.dll` (no `-a`), verify via `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` — exit code and stdout must match stock `dotnet` [effort: default] — refs REVIEW.md#F-TEST-001
- [x] 5.3 Rung 3: build or reuse the EF InMemory probe (`/tmp/ef-probe/EfApp.csproj` or `/tmp/ef-probe-out/`), run `dotnet-rs /tmp/ef-probe-out/EfApp.dll` (no `-a`), compare against stock `dotnet` exit code; record pass/fail and any new gaps not in `docs/EF_GAP_BACKLOG.md` [effort: default] — refs REVIEW.md#F-TEST-001, REVIEW.md#F-SPIKE-001
- [x] 5.4 Diagnose and fix host-mode (`no -a`) Newtonsoft probe execution stall (`dotnet-rs /tmp/nuget-probe-out/App.dll` does not complete), then re-run rung 2 parity checks [effort: high] — refs REVIEW.md#F-TEST-001 — stall root-caused and fixed (constrained-callvirt generic-substitution recursion in `callvirt_constrained`); probe no longer hangs but rung-2 parity now blocked by the new enum-reflection gap in step 5.6
- [x] 5.5 Extend differential tooling so rung 2/3 can compare host-mode runs (no `-a`) without regressing existing `-a` workflows (`scripts/diff_run.sh` currently hardcodes `-a <shared_framework_dir>`) [effort: default] — refs REVIEW.md#F-TEST-001
- [x] 5.6 Investigate the enum-field reflection blocker now exposed by the 5.4 fix: `System.Reflection.FieldInfo.GetRawConstantValue()` dispatches to the throwing abstract base (`NotSupportedException: NotSupported_AbstractNonCLS`) instead of the runtime field-info override, on the `Type.GetEnumData` → `Enum.GetNames` path reached from `Newtonsoft...EnumUtils.InitializeValuesAndNames` (hit by any Newtonsoft serialization). Determine whether reflection returns the wrong field-info runtime type or virtual dispatch fails to find the override; do NOT fix in this step — produce a root-cause writeup and a fix plan [effort: high] — refs REVIEW.md#F-TEST-001
- [x] 5.7 Implement the 5.6 fix plan: add `DotnetRs.FieldInfo.GetRawConstantValue()` override + intrinsic constant materialization for enum literal fields, then rerun rung-2 parity (`bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`) [effort: high] — refs docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md#fix-plan-future-step-not-implemented-here
- [ ] 5.8 Close the adjacent reflection completeness gap discovered during 5.6: `DotnetRs.FieldInfo.GetFieldAttributes()` currently has no implementation path (`Not implemented: no body`) and should be wired or explicitly triaged with coverage [effort: default] — refs docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md#additional-adjacent-gap-discovered-not-fixed-in-this-step
- [ ] 5.9 Diagnose and fix the newly exposed rung-2 blocker after 5.7: `System.InvalidProgramException` in `System.Globalization.CompareInfo/SortHandleCache.GetCachedSortHandle` on the `Enum.GetNames` path used by Newtonsoft enum initialization [effort: high] — refs docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md#post-57-follow-on-blocker-not-fixed-in-this-step

## Phase 6: Fail-loud Compat Handling

- [ ] 6.1 Add `probe_entry_kind(path: &Path) -> EntryKind` in `crates/dotnet-assemblies/src/host.rs` detecting NativeAOT (no CLI metadata header) and single-file bundles (bundle magic bytes in tail); call it in `run_cli()` before loading, emitting an explicit human-readable error message and exiting 1 for unsupported kinds [effort: default] — refs REVIEW.md#F-COMPAT-001

## Phase 7: Roadmap Documentation Update

- [ ] 7.1 Update `docs/USERLAND_TESTING_ROADMAP.md`: mark §10 probe 5 with spike result (✅/❌ + date + reference to `docs/EF_GAP_BACKLOG.md`); mark Option A milestones 1–2 as completed with date; add one-paragraph summary of gap backlog findings under §5 or §10 [effort: default] — refs REVIEW.md#F-DOC-001
