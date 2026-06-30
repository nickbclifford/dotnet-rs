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
| Rung 2 — Newtonsoft serialization parity | ✅ **PASS (verified 2026-06-30)** — host-mode prints `{"name":"test","value":42}` exit 42, matches stock; reflection-equality differential also matches |
| Rung 3 — EF InMemory parity | ❌ NOT met — P1 generic-substitution engine bug `Generic index 0 out of bounds` (re-confirmed 2026-06-30); **out of scope** per the spike, separate epic |
| Phase 6 — fail-loud NativeAOT / single-file | ⬜ not started (5.24 tooling, 6.1) |
| Phase 7 — roadmap doc update | ⬜ not started (7.1) |

**The branch's actual deliverable — the NuGet host runner — is complete and working, and the rung-2
stretch goal (Newtonsoft) is now met.** The only unmet rung is rung-3 (EF), gated on a core
generic-substitution engine bug that is a separate epic (see the rung-3 note in Part B). Remaining
in-scope, finishable work before merge: 5.24, 6.1, 7.1.

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

**The umbrella acceptance — NOW SATISFIED:**

- [x] 5.15 ✅ **Newtonsoft host-mode parity MET (verified 2026-06-30).** Real probe
  `./target/debug/dotnet-rs /tmp/nuget-probe-out/App.dll` prints `{"name":"test","value":42}` exit 42,
  matching stock; `bash scripts/diff_run.sh /tmp/refl_probe.cs` (reflection-equality differential) also
  passes. Unblocked by the 5.21 property-object cache exactly as diagnosed in
  `docs/NEWTONSOFT_SERIALIZATION_GAP.md`. Verified against the *real* probe, not a synthetic repro. — refs REVIEW.md#F-TEST-001

**Concrete next steps (decision = Option B "keep grinding", taken 2026-06-29):**

- [x] 5.21 ▶ **NEXT — cache reflection property objects** so repeated `GetProperties()` return
  identity/equality-stable `PropertyInfo` (mirrors the existing method/field caches). This is *the*
  confirmed `{}` root cause: Newtonsoft's `GetSerializableMembers` uses `List<MemberInfo>.Contains` /
  set ops, and uncached fresh `PropertyInfo` objects compare unequal across calls so every member is
  dropped. Full templated plan (4 edit sites + **mandatory GC-trace loop**) and the differential probe
  are in `docs/NEWTONSOFT_SERIALIZATION_GAP.md`. **Verify against the real probe**
  (`bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj` → `{"name":"test","value":42}`), not a
  synthetic repro — that drift is what produced 5.16–5.20. [effort: high] — refs REVIEW.md#F-TEST-001
- [x] 5.22 (tangential, lower priority) enum formatting via **interpolated string** (`$"{enum}"`) throws
  `InvalidCastException` in `System.Enum.TryFormatUnconstrained` / `DefaultInterpolatedStringHandler` —
  a separate path from the 5.20 `Enum.ToString()` intrinsic. Surfaced by the diagnostic probe; **not**
  on the `Item` serialization critical path. Fix opportunistically, not bundled with 5.21. [effort: default] — refs REVIEW.md#F-TEST-001
- [x] 5.23 Interpolated formatting parity follow-up: non-string interpolations currently produce blank
  stdout (e.g., `$"{1}"`, `$"{enum}"`), despite no exception after 5.22. Track as a separate VM
  formatting/runtime-contract gap (outside host-runner critical path). [effort: high] — refs REVIEW.md#F-TEST-001
- [ ] 5.24 `scripts/diff_run.sh` cargo fallback bug: when `target/debug/dotnet-rs` is absent and no
  global `dotnet-rs` exists, the script can hit an empty command invocation (`line 134: : command not
  found`) instead of falling back to `cargo run`; harden runner resolution/branching. [effort: default]
  — refs REVIEW.md#F-TEST-001

> **Rung 3 (EF InMemory) — parity UNMET; re-confirmed 2026-06-30.** Host-mode EF still fails with
> `Generic index 0 out of bounds (length 0)`. Now localized (see `docs/EF_GAP_BACKLOG.md` →
> "Rung-3 re-confirmation + localization"): a **generic static method** call
> (`ImmutableSortedDictionary.Create<Type, ValueTuple<…>>` from `DbContextOptions..ctor`) resolves
> `!!0`/`!!1` against an empty `GenericLookup` — the method-generic args aren't threaded into the
> resolution context. Same *class* as the 5.4 constrained-callvirt bug. Unlike rung-2 (a contained
> reflection gap), this is a **core engine bug** and EF is large — treat as a **separate epic**, not a
> quick follow-on. (refs REVIEW.md#F-TEST-001, REVIEW.md#F-SPIKE-001)

- [ ] 5.25 (EF epic entry — only if pursuing rung-3) Thread generic *method* arguments into the
  type-resolution context for generic static-method calls so `ImmutableSortedDictionary.Create<TKey,TValue>`
  resolves `!!0`/`!!1` correctly. Start at `crates/dotnet-runtime-resolver/src/methods.rs::find_generic_method`
  (~52–76) and verify the dispatched frame's lookup carries `method_generics` into body/`.cctor`
  type-resolution; **fix upstream propagation, do not add another `generics.rs` fallback heuristic**.
  Repro: `./target/debug/dotnet-rs /tmp/ef-probe-out/EfApp.dll` (rebuild probe per REVIEW F-SPIKE-001).
  Expect further EF blockers after this one. [effort: high] — refs REVIEW.md#F-TEST-001, REVIEW.md#F-SPIKE-001

---

## 🔀 Decision point — rung-2 DONE; next fork (open, 2026-06-30)

Rung-2 (Newtonsoft) is now met — Option B paid off via the single 5.21 property-cache fix. Rung-1 ✅,
rung-2 ✅, host runner ✅. The branch has met its deliverable **and** its stretch goal. Two ways forward:

- **Option A — wrap up and merge (recommended).** Do the small in-scope remainder — **5.24** (diff_run.sh
  fallback bug), **6.1** (fail-loud NativeAOT/single-file), **7.1** (roadmap doc) — then finalize and
  merge. Rung-3 (EF) stays a tracked gap (its localization is now recorded in `docs/EF_GAP_BACKLOG.md`),
  picked up on a dedicated branch.

- **Option B — keep grinding into rung-3 (EF).** Take **5.25** (thread generic method args into type
  resolution). Be clear-eyed: this is a **core engine bug**, not a contained reflection gap, and EF will
  surface further blockers behind it — i.e. a new multi-step epic, larger than the rung-2 chain was.
  Recommend still landing 5.24/6.1/7.1 first so the branch stays mergeable independent of the EF outcome.

> Lesson that held: verify every rung change against the **real** probe, not a minimal repro —
> `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`. 5.16–5.20 were the cautionary example; 5.21
> got it right and that's why rung-2 actually closed.

---

## ⬜ Part C — Remaining in-scope work (do regardless of the decision)

- [ ] 6.1 Add `probe_entry_kind(path) -> EntryKind` in `host.rs` detecting NativeAOT (no CLI metadata header) and single-file bundles (bundle magic in tail); call it in `run_cli()` before loading, emitting a clear human-readable error and exiting 1 for unsupported kinds — refs REVIEW.md#F-COMPAT-001
- [ ] 7.1 Update `docs/USERLAND_TESTING_ROADMAP.md`: mark §10 probe 5 with the EF spike result (❌ + date + link to `docs/EF_GAP_BACKLOG.md`); mark Option A milestones 1–2 ✅ with date; one-paragraph gap-backlog summary; note rung-2 Newtonsoft parity status per the Decision Point — refs REVIEW.md#F-DOC-001

---

## Finalize (after the decision + Part C)

- [ ] Whole-branch finalizer audit (high-effort): confirm `--assemblies` parity, `cargo fmt --all -- --check`, fixture suite green, host-mode rung-1 + rung-2 green. (DB step states reconciled 2026-06-29/30: 5.13 + 5.15 closed.)
- [ ] Remove refactor scratch artifacts (`REVIEW.md`, `CHECKLIST.md`, `AGENT_MEMORY.md`, `AGENT_PROMPT.md`) before merge to `main`.
