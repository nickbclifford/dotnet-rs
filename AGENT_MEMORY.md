# Agent Memory — NuGet Host Runner

This file is an append-only log. Every step agent appends one entry at the end using the format below.
Never edit or reformat prior entries.

## Entry format

```
## <ISO-8601 date> — <Step ID/title> — <model> — <status: completed|blocked>
**Goal:** one sentence.
**What changed:** files/modules created or modified.
**What I learned:** key findings, surprises, anything a future step needs to know.
**Follow-ups for future steps:** gotchas, ordering constraints, things to verify.
**Open questions:** anything genuinely uncertain that a later step should revisit.
```

---

## 2026-06-23 — Kickoff review — claude-sonnet-4-6 — completed

**Goal:** Plan the two-stage refactor: (Stage 1) empirically probe EF InMemory end-to-end via a
hand-assembled flat directory and produce a prioritized gap backlog; (Stage 2) implement Option A
milestones 1–2: a real `runtimeconfig.json` reader with roll-forward version selection and a
`deps.json` reader that derives managed and native probing lists, generalizing the flat-directory
loader to multi-root probing.

**What changed:** Created `REVIEW.md`, `CHECKLIST.md`, `AGENT_PROMPT.md`, `AGENT_MEMORY.md` at
repo root.

**What I learned:**
- The flat-directory loader (`loader.rs:147–159`, `load_and_register:loader.rs:286–320`) stores lazy
  entries as `HashMap<String, Option<ResolutionS>>` with the implicit contract that the path is always
  `assembly_root/{name}.dll`. Multi-root probing needs to break this implicit contract — planned fix is
  to add `probing_paths: DashMap<String, PathBuf>` and check it first in `load_and_register`.
- `find_latest_runtime_in_base(resolution.rs:871–899)` is the crude "pick highest version subdir" logic
  that roll-forward replaces. It is used by both `find_dotnet_app_path()` and `find_dotnet_sdk_path()`
  (both exported from `crates/dotnet-assemblies/src/lib.rs`). Do not delete these — the test harness
  uses `find_dotnet_app_path()` in `harness.rs:45–50`.
- EF InMemory packages are NOW in `~/.nuget/packages` after the planning probe restore (EF 9.0.17,
  Microsoft.Extensions.* 9.0.17). The spike step (1.1) can proceed immediately without a restore step.
  The flat output dir from the probe was at `/tmp/ef-probe-out/` — confirm it still exists or rebuild.
- `newtonsoft.json` IS present at `~/.nuget/packages/newtonsoft.json/13.0.3/` (and 13.0.1, 9.0.1).
  The Newtonsoft.Json probe app was built at `/tmp/nuget-probe-out/App.dll` during planning.
- Installed runtimes: `Microsoft.NETCore.App 8.0.28` and `10.0.9` at
  `/usr/share/dotnet/shared/Microsoft.NETCore.App/`. SDK: 10.0.109.
- A real fixture's `SingleFile.runtimeconfig.json` (net10.0) contains `framework.version = "10.0.0"`
  with no `rollForward` key (default = `Minor` policy). Roll-forward from `10.0.0` → `10.0.9` is
  the normal case for the installed runtime.
- A real `deps.json` with a NuGet dependency has this probing pattern: `targets[tfm][pkg/ver].runtime`
  is a map of `relative_asset_path → metadata`; `libraries[pkg/ver].path` is the NuGet package
  subdirectory (lowercase). Full path = `~/.nuget/packages/` + `libraries[pkg].path` + `/` +
  `targets[tfm][pkg].runtime.key`.
- `serde` and `serde_json` are NOT in workspace dependencies yet. The step agent for 2.1 must add
  them to both `Cargo.toml` (workspace) and `crates/dotnet-assemblies/Cargo.toml`.
- The `IsDynamicCodeSupported = false` switch is hardcoded at `state.rs:447–455` (confirmed). Do not
  touch it.
- The test harness (`harness.rs`) uses `find_dotnet_app_path()` to build the loader for all tests
  and runs the binary subprocess with `-a <dir> <dll>`. The `-a` flag must remain working with the
  same semantics.
- `diff_run.sh` always passes `-a <shared_framework_dir>` to `dotnet-rs`. After CLI step 4.1, the
  new host mode is invoked WITHOUT `-a`, not via `diff_run.sh` unless we build a new probe project
  and run diff_run.sh against the `.csproj` file.
- EF app closure (11 DLLs in `/tmp/ef-probe-out/`, excluding `EfApp.dll` entry): all `net8.0` or
  `net9.0` assets from the respective packages. The framework dir is `net10.0`. Assembly version
  mismatches are expected but tolerated (non-strict versioning by default).
- `dotnet-runtime-resolver` crate exists but is about method/type resolution at runtime, not host
  configuration. New host code belongs in `crates/dotnet-assemblies/src/host.rs`.
- `crates/dotnet-assemblies/src/lib.rs` re-exports: `AssemblyLoader`, `SUPPORT_ASSEMBLY`,
  `default_read_options`, `find_dotnet_app_path`, `find_dotnet_sdk_path`. New exports from `host.rs`
  must be added here.

**Follow-ups for future steps:**
- Step 1.1 (EF spike): create the flat dir by copying `/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.9/*.dll` + `/tmp/ef-probe-out/*.dll` into a temp dir. Run with tracing enabled (`RUST_LOG=debug` or equivalent) to get more useful output. Cap execution at 60 seconds; if it hangs during DI construction, that's the blocker.
- Steps 2.1–2.3: `serde`/`serde_json` must be added to workspace `Cargo.toml` first. Confirm the `host.rs` module is declared in `crates/dotnet-assemblies/src/lib.rs`.
- Step 3.2 (loader refactor): run the FULL fixture test suite after the refactor — `DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run --no-default-features -p dotnet-cli`. All tests must pass with no behavior change.
- Step 4.1 (CLI): `diff_run.sh` will NOT need to be changed — it passes `-a` and that path still works. But the new `dotnet-rs <dll>` (no `-a`) mode is the NEW thing being tested in Phase 5.
- Native probing list (`derive_native_search_dirs`) is derived in step 3.1 but NOT wired into the P/Invoke loader (`crates/dotnet-pinvoke/`). That is downstream Option-A milestone 4. Document this clearly in the step memory.
- `configProperties` from `runtimeconfig.json` are parsed in step 2.1 but NOT wired into `AppContext` switches. That is downstream Option-A milestone 5. The existing hardcoded `IsDynamicCodeSupported=false` in `state.rs` must remain in place.

**Open questions:**
- Will EF's DI container construction succeed at all under the current interpreter? The core bottleneck is whether `Microsoft.Extensions.DependencyInjection` (which uses heavy reflection + `Expression.Compile` for activator delegates) can execute. With `IsDynamicCodeSupported=false` routing `Compile` through the interpreter, this should be the reachable path — but reflection depth of the DI container is unknown.
- Does `dotnet build` always copy NuGet DLLs flat to the output directory for framework-dependent builds? Verified yes for our probe projects (net10.0, EF 9.0.17). Self-contained deploys work differently and are not in scope.
- The `dotnet-runtime-resolver` crate has a `resolution.rs` — does it have any host-parsing code that might conflict? Brief inspection showed it handles type/method resolution at VM runtime, not file-level host config. No conflict expected.

## 2026-06-24 — Step 1.1 EF InMemory flat-dir spike — gpt-5-codex — completed

**Goal:** Assemble the EF flat directory, run the EF probe under existing `dotnet-rs -a`, and record a prioritized no-fix gap backlog.

**What changed:** Added `docs/EF_GAP_BACKLOG.md`; checked off `CHECKLIST.md` item 1.1; added follow-up checklist item 3.4 for wiring host-derived native search directories into P/Invoke.

**What I learned:**
- Confirmed the cited `IsDynamicCodeSupported = false` switch in `crates/dotnet-vm/src/state.rs` still exists in the `app_context_switches` initialization and was not touched.
- The existing `--assemblies` path still scans a single managed assembly root and `load_and_register` still constructs `{assembly_root}/{name}.dll`; no host/probing refactor has happened yet.
- `/tmp/ef-probe-out/EfApp.dll` still exists and stock `dotnet /tmp/ef-probe-out/EfApp.dll` prints `Hello` and exits `42`.
- The required DLL-only flat dir (`/tmp/ef-flat-dir`, 184 managed DLLs from `/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.9/*.dll` plus `/tmp/ef-probe-out/*.dll`) fails quickly, not by timeout: `System.DllNotFoundException: Unable to load DLL 'libSystem.Native': dlopen failed` from `Interop.GetCryptographicallySecureRandomBytes -> System.Guid.NewGuid -> Microsoft.EntityFrameworkCore.DbContext..ctor`.
- Setting `LD_LIBRARY_PATH` to the framework runtime directory did not change the `libSystem.Native` failure. Copying the framework `.so` files into `/tmp/ef-flat-dir` allowed a deeper exploratory run; that then failed with `Internal VM error: Type resolution failed: Generic index 0 out of bounds (length 0)` during early `DbContextOptions` construction, around `ImmutableSortedDictionary.Create<System.Type, ValueTuple<IDbContextOptionsExtension,int>>`.
- `DOTNET_RS_TRACE` is the useful tracer knob, not `RUST_LOG` alone. However, with native `.so` files copied in, enabling `DOTNET_RS_TRACE` can itself panic while formatting generic debug output: `dotnetdll ... resolved/types.rs:691:45: index out of bounds: the len is 215 but the index is 754`. Untraced native-assisted execution reports the generic-index VM error instead of panicking.

**Follow-ups for future steps:**
- New checklist item 3.4 is needed if host mode is expected to run EF without manually copying framework native libraries: `derive_native_search_dirs` alone will not help until `dotnet-pinvoke::NativeLibraries` / VM state can use those directories.
- After native probing is fixed or worked around, the next EF blocker is generic method/type substitution in early EF options construction, before the app reaches provider configuration, DI/service-provider construction, `SaveChanges`, or query execution.
- Deep EF tracing may require fixing or avoiding debug formatting of cross-resolution generic type handles; otherwise `DOTNET_RS_TRACE` can mask the real execution error with a tracing-only panic.

**Open questions:**
- Is the `Generic index 0 out of bounds (length 0)` caused by cross-assembly generic substitution, by `System.Collections.Immutable` net10/net8 version skew, or by a more general constructed-generic method resolution bug? This spike did not fix or minimize it.
- Should future EF probe flat dirs include framework native libraries by convention, or should the plan wait for native-search-dir wiring in the host path? The required Step 1.1 DLL-only run proves the current command cannot progress without native library discovery.

## 2026-06-24 — Step 2.1 runtimeconfig parser scaffold — gpt-5-codex — completed

**Goal:** Add host-side `runtimeconfig.json` serde types + parser in `dotnet-assemblies`, including a unit test against a real fixture runtimeconfig.

**What changed:** Added `crates/dotnet-assemblies/src/host.rs` with `RuntimeConfig`, `RuntimeOptions`, `FrameworkRef`, `RollForwardPolicy`, `HostError`, and `parse_runtimeconfig(&Path)`; added a unit test `host::tests::parses_fixture_runtimeconfig` that parses `/tmp/fixture-probe/SingleFile.runtimeconfig.json` and asserts `tfm`, framework name/version, absent `rollForward`, and expected `configProperties` value; updated `Cargo.toml` workspace deps with `serde` + `serde_json`; updated `crates/dotnet-assemblies/Cargo.toml` to consume those deps; registered the new module in `crates/dotnet-assemblies/src/lib.rs`; checked off checklist item `2.1` in `CHECKLIST.md`.

**What I learned:** The REVIEW.md-cited runtime discovery code in `crates/dotnet-assemblies/src/resolution.rs` (`find_latest_runtime_in_base`, `find_dotnet_app_path`) still matches the described pre-host state; the `IsDynamicCodeSupported = false` app-context switch in `crates/dotnet-vm/src/state.rs` is still present and untouched. The `/tmp/fixture-probe/SingleFile.runtimeconfig.json` fixture file exists in this environment and matches the expected shape.

**Follow-ups for future steps:** Step 2.2 can now focus on roll-forward selection logic (`select_framework_version`) using `RollForwardPolicy`; step 2.3 can wire `DOTNET_ROOT` + runtimeconfig parsing into end-to-end framework resolution.

**Open questions:** The new unit test currently expects `/tmp/fixture-probe/SingleFile.runtimeconfig.json` to exist; if future environments do not precreate that probe artifact, decide whether to standardize fixture generation before `dotnet-assemblies` tests or relocate this test input under repository-controlled paths.

## 2026-06-24 — Step 2.2 roll-forward selector implementation — gpt-5-codex — completed

**Goal:** Implement host-side framework version selection with all six roll-forward policies and add focused unit coverage.

**What changed:** Updated `crates/dotnet-assemblies/src/host.rs` to add `select_framework_version(base_dir, requested, policy) -> Option<PathBuf>` plus internal helpers for parsing `major.minor.patch` directory names, scanning installed framework directories, and policy-specific candidate selection (`Disable`, `LatestPatch`, `Minor`, `LatestMinor`, `Major`, `LatestMajor`). Added unit tests that model the verified installed runtime set (`8.0.28`, `10.0.9`) for a `10.0.0` request and assert expected policy outcomes, including the `Disable` no-exact-match boundary and exact-match behavior.

**What I learned:** Before editing, the REVIEW-cited code in `crates/dotnet-assemblies/src/resolution.rs` (`find_latest_runtime_in_base`) and the hardcoded `IsDynamicCodeSupported = false` switch in `crates/dotnet-vm/src/state.rs` still matched the described state; no discrepancy to report. The new selector is isolated in `host.rs` and does not alter existing `-a/--assemblies` behavior.

**Follow-ups for future steps:** Step 2.3 can now consume `select_framework_version` when wiring `resolve_framework_from_runtimeconfig` and `DOTNET_ROOT` override behavior.

**Open questions:** The current parser accepts only strict numeric `major.minor.patch` directory names. If preview/suffix runtime folder naming must be supported later, decide whether to extend parsing rules then (out of scope for this step).

## 2026-06-24 — Step 2.3 runtimeconfig framework resolver wiring — gpt-5-codex — completed

**Goal:** Add host-side framework resolution from parsed runtimeconfig by combining framework policy selection with base-path discovery, including `DOTNET_ROOT` support.

**What changed:** Updated `crates/dotnet-assemblies/src/host.rs` to add `resolve_framework_from_runtimeconfig(config, override_base) -> Option<PathBuf>` and a small `framework_base_candidates(framework_name)` helper that checks `DOTNET_ROOT` first, then OS-default shared framework roots, before calling `select_framework_version`. Added two end-to-end unit tests using the fixture runtimeconfig at `/tmp/fixture-probe/SingleFile.runtimeconfig.json`: one validates resolution via `override_base`, and one validates `DOTNET_ROOT`-driven resolution with an isolated temp `shared/Microsoft.NETCore.App` layout. Updated `CHECKLIST.md` to mark step `2.3` complete.

**What I learned:** Before edits, the REVIEW-cited runtime discovery code in `crates/dotnet-assemblies/src/resolution.rs` (`find_latest_runtime_in_base`) and the hardcoded `IsDynamicCodeSupported = false` block in `crates/dotnet-vm/src/state.rs` still matched plan assumptions; no discrepancy to report. In this Rust toolchain/edition, env var mutation APIs in tests require `unsafe`, so the `DOTNET_ROOT` test serializes mutation via a local mutex and restores prior state.

**Follow-ups for future steps:** Step 3.x can consume `resolve_framework_from_runtimeconfig` directly when wiring `AssemblyLoader::new_from_host`; keep `--assemblies` path unchanged. If more tests begin mutating process env vars, consider consolidating env mutation helpers to avoid cross-test interference.

**Open questions:** `framework_base_candidates` currently mirrors existing `find_dotnet_app_path` conventions and only consults `DOTNET_ROOT` (not arch-specific variants such as `DOTNET_ROOT_x64`); decide later whether expanded host-env compatibility is required for this project.

## 2026-06-24 — Step 3.1 deps.json parser + probing derivation — gpt-5-codex — completed

**Goal:** Add host-side `deps.json` serde parsing plus managed/native probing derivation helpers and focused unit coverage for fixture/no-NuGet and Newtonsoft/NuGet cases.

**What changed:** Updated `crates/dotnet-assemblies/src/host.rs` with new serde types (`DepsJson`, `DepsRuntimeTarget`, `TargetLibrary`, `AssemblyAssetInfo`, `LibraryInfo`), new `HostError` variants for deps read/parse failures, `parse_deps_json(&Path) -> Result<DepsJson, HostError>`, `derive_managed_probing_paths(&DepsJson, &Path) -> Vec<(String, PathBuf)>`, `derive_native_search_dirs(&DepsJson, &Path) -> Vec<PathBuf>`, and `nuget_global_packages_dir() -> PathBuf`. Added unit tests that parse `/tmp/fixture-probe/SingleFile.deps.json` and `/tmp/nuget-probe-out/App.deps.json` and assert expected managed/native derivation behavior. Marked checklist item `3.1` complete in `CHECKLIST.md`.

**What I learned:** Before edits, REVIEW-cited locations still matched assumptions: `find_latest_runtime_in_base` in `crates/dotnet-assemblies/src/resolution.rs` is still present and unchanged, and the hardcoded `IsDynamicCodeSupported = false` switch remains in `crates/dotnet-vm/src/state.rs` untouched. The current probe artifacts at `/tmp/fixture-probe` and `/tmp/nuget-probe-out` contain deps.json shapes matching REVIEW examples.

**Follow-ups for future steps:** Step 3.2 can consume managed probing pairs to populate loader probing maps without changing existing `-a` behavior. Step 3.4 still needs to wire derived native directories into `dotnet-pinvoke`/VM native resolution; this step only derives the list.

**Open questions:** As with runtimeconfig tests, deps parser tests currently rely on `/tmp` probe artifacts existing in the environment; if this becomes flaky in CI/dev setups, future work should standardize generating these inputs before `dotnet-assemblies` tests.

## 2026-06-24 — Step 3.2 loader probing-path refactor — gpt-5-codex — completed

**Goal:** Extend `AssemblyLoader` with explicit probing-path registration and secondary-root scanning while preserving existing `--assemblies` behavior.

**What changed:** Updated `crates/dotnet-assemblies/src/loader.rs` to add `probing_paths: DashMap<String, PathBuf>` on `AssemblyLoader`, initialize it in `new_internal`, and update `load_and_register` to prefer a registered probing path before the legacy `{assembly_root}/{name}.dll` fallback. Added `register_probing_path(name: &str, path: PathBuf)` and `add_scan_root(root: &Path) -> Result<(), AssemblyLoadError>`; both register lazy externals (`None`) plus explicit probe paths only when the assembly name is not already present in `external` (preserving primary-root precedence). Added focused unit tests for probing-path registration and scan-root behavior. Marked checklist item `3.2` complete in `CHECKLIST.md`.

**What I learned:** Before edits, REVIEW-cited code still matched assumptions: `crates/dotnet-assemblies/src/resolution.rs` still contains `find_latest_runtime_in_base`, `crates/dotnet-vm/src/state.rs` still has the hardcoded `IsDynamicCodeSupported = false` switch in the same block, and `AssemblyLoader::new`/`load_and_register` were still single-root as described. No discrepancies were found.

**Follow-ups for future steps:** Step 3.3 can call `add_scan_root(entrypoint.parent())` and then `register_probing_path` for deps-derived assets without changing `-a` semantics; because registration is now non-overwriting, primary/framework and first-seen scan-root entries continue to win on name conflicts.

**Open questions:** `register_probing_path` currently ignores any name already present in `external` (including lazy `None` entries), which preserves current root precedence and avoids behavior changes; if later host policy needs explicit override ordering across multiple probing sources, that precedence contract should be specified before changing this behavior.
