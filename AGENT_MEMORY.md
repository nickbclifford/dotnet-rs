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

## 2026-06-24 — Step 3.3 new_from_host constructor + host exports — gpt-5-codex — completed

**Goal:** Add a host-aware `AssemblyLoader::new_from_host` constructor that resolves framework + probing roots from app host metadata, and re-export host-layer types from crate root.

**What changed:** Updated `crates/dotnet-assemblies/src/loader.rs` to add `AssemblyLoader::new_from_host(entrypoint: &Path, nuget_global: Option<&Path>) -> Result<Self, HostError>` with flow: derive `*.runtimeconfig.json` → parse runtimeconfig → resolve framework dir → `AssemblyLoader::new(framework_dir)` → `add_scan_root(entrypoint.parent())` → optional `*.deps.json` parse + `derive_managed_probing_paths` + `register_probing_path`. Added a focused loader unit test `new_from_host_uses_fixture_runtimeconfig_and_app_scan_root` against `/tmp/fixture-probe/SingleFile.dll`. Updated `crates/dotnet-assemblies/src/host.rs` `HostError` with constructor-wiring variants (`ResolveFramework`, `CreateAssemblyLoader`, `AddScanRoot`) so loader init/add-scan failures are surfaced as host errors. Updated `crates/dotnet-assemblies/src/lib.rs` to re-export host types/helpers at crate root. Marked checklist item `3.3` complete in `CHECKLIST.md`.

**What I learned:** Before edits, the REVIEW-cited code still matched assumptions: `find_latest_runtime_in_base` remained in `crates/dotnet-assemblies/src/resolution.rs`, and the hardcoded `IsDynamicCodeSupported = false` app-context switch remained at `crates/dotnet-vm/src/state.rs` unchanged. `new_from_host` can stay additive without affecting existing `-a` mode because it is a separate constructor.

**Follow-ups for future steps:** Step 4.1 can call `AssemblyLoader::new_from_host` directly for CLI no-`-a` mode and preserve existing `AssemblyLoader::new` path for `-a`. Step 3.4 remains required for native library probing (`libSystem.Native` etc.); this step only wires managed probing from deps.

**Open questions:** `new_from_host` currently skips deps probing if `<entrypoint>.deps.json` is absent (optional behavior per plan) but still fails hard on malformed deps when present; if host policy later requires strict deps presence, this behavior should be revisited explicitly.

## 2026-06-24 — Step 3.4 native search-dir wiring into P/Invoke/VM state — gpt-5-codex — completed

**Goal:** Wire host-derived native probing directories into runtime native library resolution so host-mode apps can resolve framework/package native assets without copying `.so` files into the managed assembly root.

**What changed:** Updated `crates/dotnet-assemblies/src/loader.rs` to track native probing directories (`native_search_dirs`), expose `register_native_search_dir` + `native_search_dirs()` accessors, and extend `AssemblyLoader::new_from_host` to register app directory + deps-derived native dirs (`derive_native_search_dirs`) alongside managed probing paths. Updated `crates/dotnet-pinvoke/src/loader.rs` `NativeLibraries` to support additional search directories (`with_search_dirs`) and search across primary root first, then host-provided dirs, before falling back to system loader paths. Updated `crates/dotnet-vm/src/state.rs` so `SharedGlobalState::new` builds `NativeLibraries` with loader-provided native search dirs. Added/updated focused tests in `dotnet-assemblies` (`new_from_host_registers_native_dirs_from_deps_assets`, plus fixture host test assertion) and in `dotnet-pinvoke` (additional-dir lookup and primary-root precedence). Marked checklist item `3.4` complete in `CHECKLIST.md`.

**What I learned:** Before edits, REVIEW-cited assumptions still matched current code: `find_latest_runtime_in_base` remained in `crates/dotnet-assemblies/src/resolution.rs`, and the hardcoded `IsDynamicCodeSupported = false` switch remained in `crates/dotnet-vm/src/state.rs` unchanged. Wiring native dirs through loader → VM state is additive and does not change `-a` semantics: legacy `AssemblyLoader::new` still uses only its assembly root unless host-mode constructor registers extra dirs.

**Follow-ups for future steps:** Step 4.1 can adopt host mode in CLI (`AssemblyLoader::new_from_host`) and automatically benefit from native-search wiring without changing the existing `-a` path. If host policy later needs explicit ordering beyond current precedence (primary root, then registration order of extra dirs), that contract should be documented before changing resolution order.

**Open questions:** Native-asset derivation still depends on the current `derive_native_search_dirs` logic from step 3.1; if broader RID fallback selection is required for packages that publish multiple RID target groups, that should be addressed as a dedicated follow-up rather than folded into this wiring step.

## 2026-06-24 — Step 4.1 make `--assemblies` optional and wire CLI host mode — gpt-5-codex — completed

**Goal:** Make CLI `--assemblies/-a` optional and route no-flag invocations through host-based loader initialization with an explicit missing-runtimeconfig error.

**What changed:** Updated `crates/dotnet-cli/src/lib.rs` so `Args.assemblies` is now `Option<String>` with clap annotation `required = false`; changed `run_cli()` to branch loader creation by mode: `Some(dir)` keeps existing `AssemblyLoader::new(dir)` behavior, while `None` computes `<entrypoint>.runtimeconfig.json`, emits `error: no --assemblies flag and no <name>.runtimeconfig.json found next to <dll>` and exits 1 when missing, otherwise calls `AssemblyLoader::new_from_host(&entrypoint_path, None)`. Also switched entrypoint loading to use `PathBuf` (`load_resolution_from_file(&entrypoint_path)`). Marked checklist item `4.1` complete in `CHECKLIST.md`.

**What I learned:** Before edits, REVIEW-cited CLI assumptions still matched current code (`Args.assemblies: String`, required `-a`, and unconditional `AssemblyLoader::new(args.assemblies)` path), so no discrepancy from plan. The `-a` harness path remains unchanged, and no-`-a` host mode successfully runs a prebuilt fixture DLL (exit 42); missing runtimeconfig now fails with the explicit targeted message and exit code 1.

**Follow-ups for future steps:** Step 5.1 can now exercise host-path fixture execution (`dotnet-rs <SingleFile.dll>` without `-a`) directly on top of this wiring while keeping existing integration harness behavior intact.

**Open questions:** None.

## 2026-06-24 — Step 5.1 Rung 1 fixture host-path validation — gpt-5-codex — completed

**Goal:** Verify host-path execution for the `expression_compile_42` fixture (no `-a`) and confirm full `dotnet-cli` fixture-suite stability.

**What changed:** Marked checklist item `5.1` complete in `CHECKLIST.md`; appended this step log entry to `AGENT_MEMORY.md`.

**What I learned:** Before running checks, the REVIEW-cited `IsDynamicCodeSupported = false` block in `crates/dotnet-vm/src/state.rs` (lines 447–455 in REVIEW context; currently in the same `app_context_switches` initialization region) remained unchanged. Rebuilt fixtures with `cargo run -p xtask -- fixtures build`, then ran `cargo run --bin dotnet-rs --no-default-features -- target/debug/dotnet-fixtures/expressions/expression_compile_42/SingleFile.dll` (no `-a`) and observed exit code `42`. Full fixture validation with `DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run --no-default-features -p dotnet-cli` passed with no regressions (`158 passed, 3 skipped`).

**Follow-ups for future steps:** Step 5.2 can proceed with the Newtonsoft.Json host-path probe/diff on top of the now-validated no-`-a` fixture baseline.

**Open questions:** None.

## 2026-06-23 — Step 5.4 host-mode Newtonsoft stall root-cause + fix — claude-opus-4-8 — completed

**Goal:** Diagnose and fix the host-mode Newtonsoft probe stall blocking step 5.2.

**What changed:** `crates/dotnet-vm/src/instructions/calls.rs` `callvirt_constrained` — the constrained-callvirt dispatch now runs a method resolved against the constraint type itself under the **constraint type's own** generic instantiation (`constraint_type_source.make_lookup()`, preserving method generics) instead of the caller frame's `lookup`. Restructured the dispatch-strategy `match` to yield `(method, dispatch_lookup)` per branch; the box/virtual-dispatch fallbacks keep using the caller `lookup` (unchanged). Added CHECKLIST steps 5.4 (done), 5.5 (host-mode diff tooling), and 5.6 (investigate the downstream enum blocker). NOTE: an orchestrator restart wiped the first write of this entry and the prior 5.2-blocked entry; this is the rewrite.

**What I learned:**
- The stall was **not** host-mode/loader-specific. It is a VM execution bug (unbounded recursion) that host mode merely *reaches* first because step 3.4 wires native search dirs, so `libSystem.Globalization.Native` loads and execution gets past `CultureInfo` init into real serialization. Reproduced identically under `-a <flat-dir>` once the framework `.so` files are copied into the flat dir (without them, `-a` fails early with `DllNotFoundException: libSystem.Globalization.Native`). So a flat dir for native-dependent probes must include the framework `.so`s, or run host mode.
- Diagnosis without the tracer (tracer still panics in dotnetdll `resolved/types.rs:691` during generic-type Debug formatting): process was 100% CPU + RSS climbing ~10 MB/s ⇒ allocating loop, not deadlock. `ptrace_scope=1` blocks `gdb -p`; run the binary **as a gdb child** (`gdb --args … & ; sleep ; kill -INT $gdbpid`) to sample. `samply` needs `perf_event_paranoid<=1` (unavailable here). All samples were in GC tracing because allocation dominated; a throttled per-instruction `eprintln` of `state().info_handle.source` (type+method name) revealed the frame stack growing without bound in `Newtonsoft.Json.Utilities.StructMultiKey\`2::GetHashCode`.
- Root cause: real instantiation is `StructMultiKey<StructMultiKey<Type,NamingStrategy>, EnumInfo>`. The IL `IL_001a: constrained. !T1 / callvirt object::GetHashCode()` correctly resolves `!T1` to the inner value-type and dispatches its `GetHashCode`, but `callvirt_constrained` passed the **caller** frame's `lookup` to `dispatch_method`. `dispatch_method` builds the new frame's generics entirely from that passed lookup (it does NOT consult `method.parent_generics`), so inside the inner frame `!T1` re-resolved to the outer `StructMultiKey<…>` and recursed forever. `find_generic_method` already derives callee type args via `ConcreteType::make_lookup()`; the constrained path was the one place that skipped it.
- Validation: `dotnet-vm` units 27/27; full fixture suite `DOTNET_USE_PREBUILT_FIXTURES=1 cargo nextest run --no-default-features -p dotnet-cli` = 158 passed / 3 skipped / **0 regressions**, including `structs_constrained_callvirt_42`. Newtonsoft probe (both modes) no longer hangs — completes in <1s.

**Follow-ups for future steps:** Step 5.6 — fixing the stall exposes the next blocker: `System.Reflection.FieldInfo.GetRawConstantValue()` dispatching to the throwing abstract base (`NotSupportedException: NotSupported_AbstractNonCLS`) instead of the runtime override, on `Type.GetEnumData` → `Enum.GetNames`, reached from `Newtonsoft…EnumUtils.InitializeValuesAndNames`. This blocks rung-2 parity (probe now exits 1 fast instead of hanging). 5.5 (host-mode diff tooling) still needed before parity can be checked end-to-end.

**Open questions:** Is the 5.6 `GetRawConstantValue` failure a reflection-object-type problem (enum literal fields surfaced as plain `FieldInfo` rather than the runtime field-info subtype) or a virtual-dispatch miss on the override? Same *class* as the 5.4 bug (base reached instead of override) but a different mechanism (reflection objects, not the dispatch instruction) — needs its own investigation.

## 2026-06-24 — Step 5.3 Rung 3 EF host-path probe check — gpt-5-codex — completed

**Goal:** Reuse or rebuild the EF InMemory probe, run host-mode `dotnet-rs` without `-a`, compare exit code to stock `dotnet`, and record pass/fail plus any newly discovered EF gaps.

**What changed:** Marked checklist item `5.3` complete in `CHECKLIST.md`; updated `docs/EF_GAP_BACKLOG.md` with a new host-path rung check section capturing commands/results and explicit pass/fail status.

**What I learned:** Before edits, the REVIEW-cited hardcoded `IsDynamicCodeSupported = false` switch in `crates/dotnet-vm/src/state.rs` was still present and unchanged in the same app-context switch initialization block (no discrepancy from plan). Reused existing probe artifacts at `/tmp/ef-probe/EfApp.csproj` and `/tmp/ef-probe-out/EfApp.dll` (rebuild not required). Stock `dotnet /tmp/ef-probe-out/EfApp.dll` still prints `Hello` and exits `42`. Host-mode `dotnet-rs /tmp/ef-probe-out/EfApp.dll` (no `-a`) exits `1` with `Internal VM error: Type resolution failed: Generic index 0 out of bounds (length 0)`. This is the same P1 generic-resolution blocker already documented in `docs/EF_GAP_BACKLOG.md`; no new distinct EF gap surfaced in this rung.

**Follow-ups for future steps:** Step 5.5 (host-mode diff tooling) remains needed for one-command host parity checks; current rung confirmation used direct stock/dotnet-rs command comparison. Generic-resolution investigation/fix remains outside this step and is still the EF execution blocker.

**Open questions:** None new for this step.

## 2026-06-24 — Step 5.5 host/assemblies differential tooling modes — gpt-5-codex — completed

**Goal:** Extend `scripts/diff_run.sh` so rung 2/3 can run host-mode parity checks (no `-a`) while preserving explicit `-a` differential workflows.

**What changed:** Updated `scripts/diff_run.sh` to support mode selection via `--host` and `--assemblies` flags (plus `DOTNET_RS_DIFF_MODE=host|assemblies`), changed default mode to host (no `-a`), and kept the previous `-a <shared_framework_dir>` behavior behind `--assemblies`. Updated usage text and PASS/FAIL output to include mode, and marked checklist item `5.5` complete in `CHECKLIST.md`.

**What I learned:** Before editing, the REVIEW-cited hardcoded app-context switch (`IsDynamicCodeSupported = false`) remained unchanged in `crates/dotnet-vm/src/state.rs`, and `scripts/diff_run.sh` still hardcoded `-a <shared_framework_dir>` (matching the plan’s stated pre-step discrepancy). The updated script now runs the unchanged rung commands in host mode by default (`bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`) while still allowing legacy assemblies-root differentials with `--assemblies`.

**Follow-ups for future steps:** Step 5.2 can now use `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` to exercise host mode directly; the current expected failure there is the known 5.6 enum-reflection blocker, not tooling mode mismatch. If any existing workflow still depends on old default `-a`, invoke `scripts/diff_run.sh --assemblies ...` (or set `DOTNET_RS_DIFF_MODE=assemblies`).

**Open questions:** None.

## 2026-06-24 — Step 5.6 enum-field reflection blocker investigation — gpt-5-codex — completed

**Goal:** Determine whether the Newtonsoft `FieldInfo.GetRawConstantValue()` failure is caused by wrong reflection runtime field-object type vs. a virtual-dispatch miss, and produce a root-cause writeup + fix plan without implementing the fix.

**What changed:** Added investigation writeup `docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md` (repro evidence, root cause, and fix plan); marked checklist item `5.6` complete in `CHECKLIST.md`; added follow-up checklist items `5.7` (implement `GetRawConstantValue` support) and `5.8` (adjacent `GetFieldAttributes` no-body gap).

**What I learned:** Before editing, the REVIEW-cited hardcoded `IsDynamicCodeSupported = false` switch in `crates/dotnet-vm/src/state.rs` was still present unchanged in the same app-context switch initialization block. Reproducing rung 2 (`bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`) still throws `NotSupported_AbstractNonCLS` at `System.Reflection.FieldInfo.GetRawConstantValue()` on `Type.GetEnumData`. A minimal enum probe confirms stock runtime returns `System.Reflection.MdFieldInfo` and succeeds, while dotnet-rs returns `DotnetRs.FieldInfo` and throws from `GetRawConstantValue`. Code inspection shows field reflection objects are explicitly created as `DotnetRs.FieldInfo` (`dotnet-intrinsics-reflection/src/common.rs`), `DotnetRs.FieldInfo` does not override `GetRawConstantValue` (`support/FieldInfo.cs`), and no field intrinsic exists for that method (`dotnet-intrinsics-reflection/src/fields.rs`). Therefore this is a reflection runtime-type implementation gap (missing override/intrinsic), not a callvirt override lookup failure. While probing adjacent behavior, `DotnetRs.FieldInfo.GetFieldAttributes()` was also found to have no implementation path (`Not implemented: no body`), which is separate from the Newtonsoft blocker but a real reflection completeness gap.

**Follow-ups for future steps:** Implement checklist item `5.7` exactly as the writeup plan (support override + intrinsic constant materialization + rung-2 diff parity rerun). Then triage/implement `5.8` for `GetFieldAttributes` (and potentially broader `DotnetRs.FieldInfo` internal-call completeness) so callers using `FieldInfo.Attributes` do not crash with no-body errors.

**Open questions:** For `GetRawConstantValue` implementation shape, should the first pass support only enum-needed literal primitives (enough for rung 2) or full metadata-constant coverage (`bool/char/integers/floats/string/null`) to avoid near-term follow-on failures?

## 2026-06-24 — Step 5.7 GetRawConstantValue override + intrinsic constant materialization — gpt-5-codex — completed

**Goal:** Implement the 5.6 fix plan by adding `DotnetRs.FieldInfo.GetRawConstantValue()` support (override + intrinsic constant materialization) and rerun rung-2 Newtonsoft parity.

**What changed:** Updated `crates/dotnet-assemblies/src/support/FieldInfo.cs` to override `GetRawConstantValue()` as an internal call surface; updated `crates/dotnet-intrinsics-reflection/src/fields.rs` to add `DotnetRs.FieldInfo::GetRawConstantValue()` intrinsic that resolves metadata constants and materializes managed objects (`bool/char/integer/float/string/null`) with boxed primitives and proper non-literal `InvalidOperationException`; updated `CHECKLIST.md` to mark `5.7` complete and added follow-up item `5.9`; updated `docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md` with a new post-5.7 downstream blocker section.

**What I learned:** Before edits, the REVIEW-cited hardcoded `IsDynamicCodeSupported = false` switch was still present unchanged in `crates/dotnet-vm/src/state.rs` in the same `app_context_switches` initialization block (line numbers have shifted since REVIEW, but the exact switch/value are unchanged). The `GetRawConstantValue` fix is effective: a minimal enum probe now reports `DotnetRs.FieldInfo` and successfully returns raw constant values (e.g., `System.Byte` value `1`, exit `42`). Re-running rung-2 parity no longer hits `NotSupported_AbstractNonCLS`; instead it now fails later with `System.InvalidProgramException` in `System.Globalization.CompareInfo/SortHandleCache.GetCachedSortHandle`, reached via `Type.GetEnumData -> Enum.GetNames -> Newtonsoft EnumUtils`.

**Follow-ups for future steps:** New checklist item `5.9` tracks the newly exposed rung-2 blocker (`CompareInfo` invalid-program failure) and should be handled after or alongside `5.8` reflection completeness work.

**Open questions:** Is the post-5.7 `CompareInfo/SortHandleCache` `InvalidProgramException` rooted in an interpreter IL validity bug (e.g., instruction semantics/verification mismatch) or in a missing intrinsic/runtime-coupled path in globalization code that now becomes reachable?

## 2026-06-25 — Step 5.10 rung-2 parity verification rerun — gpt-5-codex — completed

**Goal:** Re-run rung-2 differential parity after steps 5.4–5.9 and record whether host-mode Newtonsoft now matches stock `dotnet`.

**What changed:** Marked checklist item `5.10` complete in `CHECKLIST.md`; added follow-up checklist item `5.11` for the newly surfaced post-5.9 failure; appended this step entry to `AGENT_MEMORY.md`.

**What I learned:** Before edits, the REVIEW-cited hardcoded `IsDynamicCodeSupported = false` switch is still present unchanged in `crates/dotnet-vm/src/state.rs` (`app_context_switches` initialization, currently around line 451). Rerunning `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` in host mode still fails parity: stock `dotnet` exits `42` and prints full expected output, while `dotnet-rs` prints only the first line (`2`) and exits `101` due to a panic. Repro is deterministic on both the diff script and direct host run (`target/debug/dotnet-rs /tmp/nuget-probe-step510/App.dll`). With `RUST_BACKTRACE=1`, the panic is `Invalid magic in ObjectInner` from `dotnet-value/src/validation.rs`, reached via `ObjectRef::read_branded -> RawMemoryAccess::read_unaligned -> dotnet-vm instructions::objects::fields::ldfld` (`fields.rs` around line 149), indicating a newly exposed object-field read/corruption issue in runtime memory/object handling during Newtonsoft execution.

**Follow-ups for future steps:** Execute new checklist item `5.11` to root-cause and fix the `Invalid magic in ObjectInner` panic (likely in `ldfld`/heap object ref read path or a preceding write that corrupts object-slot contents), then rerun rung-2 diff parity.

**Open questions:** Is this failure a regression introduced by the 5.9 opcode/object-path changes (enum coercion adjunct fixes including `ldfld` string intercept/unbox.any/PInvoke enum return handling), or a pre-existing latent heap/object validation issue only reached now that rung-2 proceeds further?

## 2026-06-25 — Step 5.12 rung-2 prerequisite drift fix (`ParameterInfo::GetMember`) — gpt-5-codex — completed

**Goal:** Resolve/triage the fresh no-default rung-2 drift where execution panicked early with `unhandled ParameterInfo intrinsic: GetMember`, so 5.11 memory-corruption debugging can proceed from the intended baseline.

**What changed:** Updated `crates/dotnet-intrinsics-reflection/src/parameters.rs` to implement the missing `DotnetRs.ParameterInfo::GetMember()` intrinsic arm by resolving the owning runtime method from `method_index` and returning its cached reflection `MemberInfo` object (`DotnetRs.MethodInfo` / `DotnetRs.ConstructorInfo` via `get_runtime_method_obj`). Marked checklist item `5.12` complete in `CHECKLIST.md`.

**What I learned:** Before edits, the REVIEW-cited hardcoded `IsDynamicCodeSupported = false` app-context switch in `crates/dotnet-vm/src/state.rs` remained present and unchanged in the same initialization block, and `parameters.rs` still had a `GetMember` intrinsic signature but no `match` arm (falling into `unreachable!`). Running a fresh no-default host-mode probe (`cargo run --bin dotnet-rs --no-default-features -- /tmp/nuget-probe-out/App.dll`) after the fix no longer fails on `ParameterInfo::GetMember`; execution now reaches the later known rung-2 failure (`Invalid magic in ObjectInner` panic), restoring the post-5.10 baseline needed for 5.11.

**Follow-ups for future steps:** Resume checklist item `5.11` memory/object corruption root-cause work from this restored baseline (`Invalid magic in ObjectInner` in `ldfld/read_unaligned`) and rerun rung-2 differential parity once fixed.

**Open questions:** None.
