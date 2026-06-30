# CHECKLIST — NuGet Host Runner (Option A Milestones 1–2 + EF Spike)

> Branch: `supervised/nuget-host-runner`. Rewritten 2026-06-29 to reflect actual state
> after Phase 5 ballooned into an open-ended VM/reflection debugging effort.

---

## Ultimate goal

Make `dotnet-rs <app.dll>` run a framework-dependent .NET app with **no `-a` flag**, by reading the
machine-readable metadata `dotnet build` already emits next to the app:

- `*.runtimeconfig.json` → select the shared framework with correct roll-forward.
- `*.deps.json` → probe managed assemblies and native assets across roots (framework dir, app output
  dir, NuGet global cache).

…while `--assemblies <DIR>` keeps working **exactly** as before (the integration harness depends on it).
Two supporting deliverables: an **EF InMemory spike** (record gaps, fix nothing) and a **3-rung test
ladder** (fixtures → Newtonsoft → EF) that *demonstrates* the host runner end-to-end.

---

## Status at a glance

| Area | Status |
|---|---|
| **Host-runner core** (runtimeconfig + roll-forward + deps.json + multi-root probing + CLI host mode) | ✅ **DONE & verified** |
| Hard constraint: `--assemblies` unchanged | ✅ Holds — fixture suite **158/158** green via the harness's `-a` path (2026-06-29) |
| EF spike → `docs/EF_GAP_BACKLOG.md` | ✅ DONE |
| Rung 1 — fixtures via host path (no `-a`) | ✅ PASS (re-verified 2026-06-29) |
| Rung 2 — Newtonsoft serialization parity | ❌ **NOT met** — still prints `{}` not `{"name":"test","value":42}` (exit 42 matches) |
| Rung 3 — EF InMemory parity | ❌ NOT met — known P1 `Generic index 0 out of bounds`, **out of scope** per the spike |
| Phase 6 — fail-loud NativeAOT / single-file | ⬜ not started |
| Phase 7 — roadmap doc update | ⬜ not started |

**The branch's actual deliverable — the NuGet host runner — is complete and working.** What is *not*
done is full execution correctness for reflection-heavy library code (Newtonsoft, EF). That is a
VM/reflection-completeness problem, **not** a host-loading problem; see the Decision Point below.

---

## ✅ Part A — Host-runner deliverable (DONE & verified)

These are the steps that fulfill the ultimate goal. All complete; rung-1 re-verified 2026-06-29.

- [x] 1.1 EF InMemory flat-dir spike → prioritized gap backlog in `docs/EF_GAP_BACKLOG.md` (read-only; fixes nothing) — refs REVIEW.md#F-SPIKE-001
- [x] 2.1 `runtimeconfig.json` serde types + `parse_runtimeconfig()` in `crates/dotnet-assemblies/src/host.rs`; serde deps wired — refs REVIEW.md#F-HOST-001
- [x] 2.2 `select_framework_version()` — all six roll-forward policies, unit-tested against installed 8.0.28 / 10.0.9 — refs REVIEW.md#F-HOST-002
- [x] 2.3 `resolve_framework_from_runtimeconfig()` (parse + select + `DOTNET_ROOT`), e2e unit test — refs REVIEW.md#F-HOST-001, REVIEW.md#F-HOST-002
- [x] 3.1 `deps.json` serde types + `parse_deps_json()` + `derive_managed_probing_paths()` + `derive_native_search_dirs()` + `nuget_global_packages_dir()` — refs REVIEW.md#F-HOST-003
- [x] 3.2 `probing_paths: DashMap` on `AssemblyLoader`; `load_and_register` checks it first; `add_scan_root()` + `register_probing_path()`; no behavior change for existing callers — refs REVIEW.md#F-LOAD-001, REVIEW.md#F-LOAD-002
- [x] 3.3 `AssemblyLoader::new_from_host(entrypoint, nuget_global)` constructor; `host.rs` types re-exported — refs REVIEW.md#F-LOAD-003
- [x] 3.4 Host-derived native search dirs wired into `dotnet-pinvoke::NativeLibraries` / VM state (framework `.so`s resolve without manual copying) — refs REVIEW.md#F-HOST-003
- [x] 4.1 `Args.assemblies: Option<String>`; `run_cli()` auto-detects host mode from `<name>.runtimeconfig.json` when `-a` absent; clear error if neither present — refs REVIEW.md#F-CLI-001
- [x] 5.1 **Rung 1**: `expression_compile_42` via host path (no `-a`) → exit 42; full suite `158 passed, 3 skipped, 0 regressions` — refs REVIEW.md#F-TEST-001
- [x] 5.5 Differential tooling: `--host` / `--assemblies` modes in `scripts/diff_run.sh` (default host); legacy `-a` workflows preserved — refs REVIEW.md#F-TEST-001

---

## ⚠️ Part B — Rung-2/3 execution-parity track (open; scope question — see Decision Point)

**What happened.** Rung 2 was meant to *verify* the host runner by running a real NuGet app
(Newtonsoft.Json) end-to-end. The host runner loads and runs it fine — but the VM's reflection/runtime
behavior is not yet complete enough to produce correct serializer output. Each fix below uncovered the
next gap, and the chain drifted away from the acceptance test: **steps 5.16–5.20 were verified only
against synthetic minimal repros (`jsonprimitive-probe`, `enum-invoke-probe`, `step51x-min`), not
against the actual Newtonsoft probe.** As of 2026-06-29 the real probe still diverges.

**Standing acceptance test (UNMET):**
```
bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj   # or run App.dll directly
# stock dotnet : {"name":"test","value":42}   exit 42
# dotnet-rs    : {}                            exit 42   ← stdout still wrong
```

Individual fixes that *did* land (each preserves the 158/158 fixture baseline — these are real,
valuable VM improvements, just not sufficient for rung-2 parity):

- [x] 5.2 First host-mode Newtonsoft run — load works; execution stalled (opened the debug chain) — refs REVIEW.md#F-TEST-001
- [x] 5.4 Fixed constrained-`callvirt` generic-substitution infinite recursion (`StructMultiKey<…>::GetHashCode`) — refs REVIEW.md#F-TEST-001
- [x] 5.6 Root-cause writeup `docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md` (`FieldInfo.GetRawConstantValue` hits throwing abstract base) — refs REVIEW.md#F-TEST-001
- [x] 5.7 `DotnetRs.FieldInfo.GetRawConstantValue()` override + enum-literal constant materialization — refs docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md
- [x] 5.8 Reflection completeness: `GetFieldAttributes`, `GetValue(object)`, `GetCustomAttributes`, `IsDefined`, `Type.GetEnumUnderlyingType`, `IsGenericTypeDefinition`, `ParameterInfo.Member`, intrinsic signature parser — refs docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md
- [x] 5.9 `StackValue::coerce_enum_to_underlying()` at affected opcode sites (ECMA-335 §III.1.1.1); `String.Equals(StringComparison)`, `unbox.any` fast-path, `ldfld` string-intercept, PInvoke enum return — refs docs/NEWTONSOFT_ENUM_REFLECTION_BLOCKER.md
- [x] 5.11 GC write barrier for heap-backed atomic mutations (fixed intermittent `Invalid magic in ObjectInner`) — refs REVIEW.md#F-TEST-001
- [x] 5.12 `DotnetRs.ParameterInfo::GetMember()` intrinsic (restored post-5.10 baseline) — refs REVIEW.md#F-TEST-001
- [x] 5.13 `handle_get_properties` real property enumeration + `GetPropertyImpl` — **landed in-tree** (`crates/dotnet-intrinsics-reflection/src/types/type_members.rs`); orchestrator DB still tags it `blocked` (stale — close it) — refs REVIEW.md#F-TEST-001
- [x] 5.14 Generic interface-virtual lookup: `ICollection<T>.CopyTo` on `List<PropertyInfo>` (`Method not found` removed) — refs REVIEW.md#F-TEST-001
- [x] 5.16 Enum value-type → scalar field coercion in `extract_int/long/native_int` (fixed `JsonPrimitiveContract::.ctor`/`set_TypeCode` AccessViolation **on the synthetic repro**) — refs REVIEW.md#F-TEST-001
- [x] 5.17 `MethodBase.Invoke` arg unboxing uses invoked-method context (`lookup.make_concrete`); `MethodInfo.Attributes`, `RuntimeType.BaseType` for intrinsic types — refs REVIEW.md#F-TEST-001
- [x] 5.18 Invoke-return bookkeeping: consume `awaiting_invoke_return` from caller frame; `void` → `null` exactly once — refs REVIEW.md#F-TEST-001
- [x] 5.19 `System.Object::GetType()` accepts `ManagedPtr`/`ValueType` receivers (fixed enum-setter invoke panic) — refs REVIEW.md#F-TEST-001
- [x] 5.20 `System.Enum::ToString()` intrinsic (fixed `InvalidCastException` on `Console.WriteLine(enum)`) — refs REVIEW.md#F-TEST-001

**The actual open blocker (the umbrella these fixes were chasing):**

- [ ] 5.15 ❗ **Newtonsoft host-mode stdout parity still UNMET** — real probe prints `{}` not
  `{"name":"test","value":42}`. Umbrella acceptance for rung-2; stays open until
  `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` prints the correct JSON. Root cause is now
  **diagnosed and confirmed empirically** — see `docs/NEWTONSOFT_SERIALIZATION_GAP.md` and step 5.21. — refs REVIEW.md#F-TEST-001

**Concrete next steps (decision = Option B "keep grinding", taken 2026-06-29):**

- [ ] 5.21 ▶ **NEXT — cache reflection property objects** so repeated `GetProperties()` return
  identity/equality-stable `PropertyInfo` (mirrors the existing method/field caches). This is *the*
  confirmed `{}` root cause: Newtonsoft's `GetSerializableMembers` uses `List<MemberInfo>.Contains` /
  set ops, and uncached fresh `PropertyInfo` objects compare unequal across calls so every member is
  dropped. Full templated plan (4 edit sites + **mandatory GC-trace loop**) and the differential probe
  are in `docs/NEWTONSOFT_SERIALIZATION_GAP.md`. **Verify against the real probe**
  (`bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` → `{"name":"test","value":42}`), not a
  synthetic repro — that drift is what produced 5.16–5.20. [effort: high] — refs REVIEW.md#F-TEST-001
- [ ] 5.22 (tangential, lower priority) enum formatting via **interpolated string** (`$"{enum}"`) throws
  `InvalidCastException` in `System.Enum.TryFormatUnconstrained` / `DefaultInterpolatedStringHandler` —
  a separate path from the 5.20 `Enum.ToString()` intrinsic. Surfaced by the diagnostic probe; **not**
  on the `Item` serialization critical path. Fix opportunistically, not bundled with 5.21. [effort: default] — refs REVIEW.md#F-TEST-001

> **Rung 3 (EF InMemory) — parity UNMET, by design.** Step 5.3 (completed) *recorded* the result:
> host-mode EF fails with `Generic index 0 out of bounds (length 0)`, the known P1 generic-resolution
> blocker in `docs/EF_GAP_BACKLOG.md`. The spike explicitly does **not** fix this — it is a tracked
> gap, not branch work. (refs REVIEW.md#F-TEST-001, REVIEW.md#F-SPIKE-001)

---

## 🔀 Decision point — DECIDED 2026-06-29: Option B (keep grinding rung-2)

> **Chosen: Option B.** Continue rung-2 from the diagnosed root cause — **step 5.21** (property-object
> caching) is the immediate next action; see `docs/NEWTONSOFT_SERIALIZATION_GAP.md`. Part C (6.1, 7.1)
> is still worth doing and is independent of the rung-2 grind. Original options kept below for context.

The host-runner deliverable (Part A) is complete. Rung-2/3 parity (Part B) is gated on
reflection/runtime-execution completeness that is **out of the original "NuGet host runner" scope** and
is open-ended (every fix has surfaced the next). Two ways forward:

- **Option A — draw the line and ship (recommended).** Declare the host runner done. Demote the
  remaining rung-2/3 parity work to a tracked gap document (mirroring `docs/EF_GAP_BACKLOG.md`) titled
  e.g. `docs/NEWTONSOFT_SERIALIZATION_GAP.md`, capturing the `{}` root cause (member-equality in
  `GetSerializableMembers`) and the fixes already landed. Then finish Part C (6.1, 7.1), finalize, and
  merge. Rung 1 is the parity proof the host runner needs; rung 2/3 become follow-up branches.

- **Option B — keep grinding rung-2.** Continue the chain at **5.15**: make reflection member objects
  (`PropertyInfo`/`MemberInfo`) compare equal across repeated `GetProperties()` calls so
  `GetSerializableMembers` stops dropping them. Expect further blockers after that; rung-3 (EF) is
  deeper still. Only choose this if full library-execution fidelity is a goal of *this* branch.

> Re-verify after **every** rung-2 change with the real probe, not a minimal repro:
> `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`. A green synthetic repro is necessary but not
> sufficient — 5.16–5.20 are the cautionary example.

---

## ⬜ Part C — Remaining in-scope work (do regardless of the decision)

- [ ] 6.1 Add `probe_entry_kind(path) -> EntryKind` in `host.rs` detecting NativeAOT (no CLI metadata header) and single-file bundles (bundle magic in tail); call it in `run_cli()` before loading, emitting a clear human-readable error and exiting 1 for unsupported kinds — refs REVIEW.md#F-COMPAT-001
- [ ] 7.1 Update `docs/USERLAND_TESTING_ROADMAP.md`: mark §10 probe 5 with the EF spike result (❌ + date + link to `docs/EF_GAP_BACKLOG.md`); mark Option A milestones 1–2 ✅ with date; one-paragraph gap-backlog summary; note rung-2 Newtonsoft parity status per the Decision Point — refs REVIEW.md#F-DOC-001

---

## Finalize (after the decision + Part C)

- [ ] Whole-branch finalizer audit (high-effort): confirm `--assemblies` parity, `cargo fmt --all -- --check`, fixture suite green, host-mode rung-1 green; reconcile orchestrator DB step states (5.13/5.15 tags) with reality.
- [ ] Remove refactor scratch artifacts (`REVIEW.md`, `CHECKLIST.md`, `AGENT_MEMORY.md`, `AGENT_PROMPT.md`) before merge to `main`.
