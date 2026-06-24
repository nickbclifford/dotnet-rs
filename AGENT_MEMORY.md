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
