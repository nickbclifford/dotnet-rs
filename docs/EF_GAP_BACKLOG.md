# EF InMemory Gap Backlog

Date: 2026-06-24

## Scope

This is the Step 1.1 EF spike backlog. I assembled a flat directory from the installed .NET shared framework plus the EF probe build output and ran the existing `--assemblies`/`-a` mode without fixing runtime gaps.

Probe app:

- Project: `/tmp/ef-probe/EfApp.csproj`
- Entrypoint: `/tmp/ef-probe-out/EfApp.dll`
- Stock `dotnet` result: prints `Hello` and exits `42`
- Probe shape: creates a `DbContext`, configures EF InMemory in `OnConfiguring`, adds one `Blog`, calls `SaveChanges`, materializes a LINQ query, prints the title, exits `42` if one result is found.

Flat directory:

- Path used: `/tmp/ef-flat-dir`
- Framework DLL source: `/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.9/*.dll`
- EF closure DLL source: `/tmp/ef-probe-out/*.dll`
- Managed DLL count: `184`

Primary command:

```bash
timeout 60s env RUST_LOG=debug \
  cargo run --bin dotnet-rs --no-default-features -- \
  -a /tmp/ef-flat-dir /tmp/ef-probe-out/EfApp.dll
```

The run completed quickly; it did not hang or time out. `RUST_LOG=debug` did not add useful VM tracing for this path.

## Prioritized backlog

### P1 — Native framework library probing blocks the exact DLL-only flat-dir run

**Observed in the required run:**

```text
Unhandled Exception: System.DllNotFoundException: Unable to load DLL 'libSystem.Native': dlopen failed
Stack Trace:
   at Interop.GetCryptographicallySecureRandomBytes(byte* buffer, int length) in IP 2
   at System.Guid.NewGuid() in IP 3
   at Microsoft.EntityFrameworkCore.DbContext..ctor(Microsoft.EntityFrameworkCore.DbContextOptions options) in IP 4
   at Microsoft.EntityFrameworkCore.DbContext..ctor() in IP 2
   at EfProbe.BlogContext..ctor() in IP 4
   at EfProbe.Program.Main() in IP 1
```

**Impact:** The exact Step 1.1 flat directory was specified as framework DLLs plus EF closure DLLs. That is sufficient for managed assembly loading, but not for P/Invoke. EF reaches `DbContext` construction and the BCL calls `Guid.NewGuid()`, which needs the framework native library `libSystem.Native.so`. Because the native library is not in the loader's native root, execution stops before EF's InMemory provider, model building, DI/service-provider construction, `SaveChanges`, or query execution.

**Priority rationale:** This is the first engine blocker for the required command. Any future host-mode EF rung must either place native runtime libraries where `dotnet-pinvoke` can find them or wire host-derived native search directories into the P/Invoke loader.

**Notes:** Adding `LD_LIBRARY_PATH=/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.9` did not change this failure. Copying the framework `.so` files into `/tmp/ef-flat-dir` did allow execution to proceed to the next blocker, confirming that the immediate failure is native library discovery from the P/Invoke root rather than managed assembly resolution.

### P1 — Generic type/method resolution fails shortly after native probing is manually unblocked

**How observed:** For deeper exploration only, I copied `/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.9/*.so` into `/tmp/ef-flat-dir` and reran the same `dotnet-rs -a /tmp/ef-flat-dir /tmp/ef-probe-out/EfApp.dll` command.

**Observed output:**

```text
Internal VM error: Type resolution failed: Generic index 0 out of bounds (length 0)
```

**Trace context from an additional traced run:** The failing area is still early `DbContext` construction, before the app reaches `OnConfiguring`/`UseInMemoryDatabase` or user-visible database operations. The trace immediately before failure includes:

```text
→ CALL void EfProbe.BlogContext::.ctor()
  → CALL void Microsoft.EntityFrameworkCore.DbContext::.ctor()
    → CALL void Microsoft.EntityFrameworkCore.DbContextOptions`1::.ctor()
      → CALL void Microsoft.EntityFrameworkCore.DbContextOptions::.ctor()
        → CALL static void Microsoft.EntityFrameworkCore.Internal.TypeFullNameComparer::.cctor()
        → CALL [Generic(2)] static System.Collections.Immutable.ImmutableSortedDictionary`2<M0, M1> System.Collections.Immutable.ImmutableSortedDictionary::Create([System.Runtime]System.Collections.Generic.IComparer`1<M0>)
```

The generic instantiation being resolved at that point includes approximately:

```text
M0 = System.Type
M1 = System.ValueTuple<Microsoft.EntityFrameworkCore.Infrastructure.IDbContextOptionsExtension, int>
```

**Impact:** After native probing is manually unblocked, the next blocker is generic resolution/substitution around EF's use of `System.Collections.Immutable.ImmutableSortedDictionary.Create<TKey,TValue>`. This prevents even default `DbContextOptions` construction from completing, so the probe does not make progress to EF service provider/DI construction.

**Priority rationale:** This is an interpreter/type-resolution engine blocker. EF cannot proceed past its options constructor until generic method/type substitution handles this pattern.

### P2 — Binding-version mismatch warnings are noisy and may hide real compatibility issues

**Observed during traced/native-assisted runs:**

```text
Binding mismatch: assembly System.Runtime has version 10.0.0.0, but 8.0.0.0 was requested (and no redirect matched)
Binding mismatch: assembly System.Collections.Immutable has version 10.0.0.0, but 8.0.0.0 was requested (and no redirect matched)
Binding mismatch: assembly System.Private.CoreLib has version 10.0.0.0, but 0.0.0.0 was requested (and no redirect matched)
```

**Impact:** The EF 9.0.17 assets in this probe are mostly net8.0/net9.0 assets running against the installed net10.0 shared framework. The current non-strict version behavior tolerates this, and these warnings were not the direct stopping condition. However, they make traces noisy and may obscure a future real binding problem.

**Priority rationale:** Not the current blocker, but worth tracking as a reflection/loader diagnostic correctness issue once the P1 blockers are cleared.

### P3 — Enabling `DOTNET_RS_TRACE` can introduce a tracing-only panic while formatting generic debug output

**Observed with native `.so` files copied into the flat directory and tracing enabled:**

```bash
timeout 60s env RUST_BACKTRACE=1 \
  DOTNET_RS_TRACE=/tmp/ef-dotnet-rs-trace-bt.log \
  DOTNET_RS_TRACE_LEVEL=debug \
  cargo run --bin dotnet-rs --no-default-features -- \
  -a /tmp/ef-flat-dir /tmp/ef-probe-out/EfApp.dll
```

**Observed panic:**

```text
thread 'main' panicked at .../dotnetdll-.../src/resolved/types.rs:691:45:
index out of bounds: the len is 215 but the index is 754
```

Backtrace excerpt:

```text
<dotnetdll::resolution::Resolution as core::ops::index::Index<dotnetdll::resolution::TypeIndex>>::index
<dotnetdll::resolved::types::UserType as dotnetdll::resolved::ResolvedDebug>::show
<dotnet_types::generics::ConcreteType as core::fmt::Debug>::fmt
dotnet_assemblies::resolution::<impl AssemblyLoader>::locate_method
```

**Impact:** This appears to be a diagnostics/debug-formatting panic caused by trace formatting of a generic type handle from a different resolution context. It is not the untraced execution result; without `DOTNET_RS_TRACE`, the same native-assisted run returns the P1 `Generic index 0 out of bounds (length 0)` VM error instead of panicking. It does make deep EF tracing less reliable.

**Priority rationale:** Instrumentation issue, lower priority than the execution blockers, but it impairs future gap discovery.

## Current EF status

- Stock `dotnet`: ✅ prints `Hello`, exits `42`.
- `dotnet-rs -a` with the required DLL-only flat dir: ❌ stops at `System.DllNotFoundException` for `libSystem.Native` during `Guid.NewGuid()` in `DbContext` construction.
- `dotnet-rs -a` with framework native `.so` files manually copied into the flat dir: ❌ stops at generic type/method resolution in early `DbContextOptions` construction.
- No hang observed. The 60-second timeout did not fire.
- No evidence yet that execution reaches EF InMemory provider operations, DI/service-provider construction, `SaveChanges`, or LINQ query materialization.

## Captured log files

These are in `/tmp` and may be ephemeral:

- `/tmp/ef-dotnet-rs-run1.log` — required DLL-only flat-dir run; `libSystem.Native` failure.
- `/tmp/ef-dotnet-rs-run3-withso.log` — native-assisted run; generic index VM error.
- `/tmp/ef-dotnet-rs-run5-trace-bt.log` — traced native-assisted run with backtrace; tracing-only debug-format panic.
- `/tmp/ef-dotnet-rs-trace.log`, `/tmp/ef-dotnet-rs-trace-bt.log` — trace files from native-assisted runs.

## Host-path rung check update (Step 5.3, 2026-06-24)

Rung 3 was rerun in host mode (no `-a`) using the existing probe output.

Commands:

```bash
dotnet /tmp/ef-probe-out/EfApp.dll
./target/debug/dotnet-rs /tmp/ef-probe-out/EfApp.dll
```

Observed results:

- Stock `dotnet`: prints `Hello`, exits `42`.
- Host-mode `dotnet-rs` (no `-a`): exits `1` with
  `Internal VM error: Type resolution failed: Generic index 0 out of bounds (length 0)`.

Pass/fail status for rung 3: **FAIL** (exit code mismatch).

New-gap check vs this backlog: **no new distinct blocker identified**. The observed host-mode failure matches the existing P1 generic-resolution blocker already documented above; native probing is no longer the first failure in host mode because step 3.4 wired native search directories.

## Rung-3 re-confirmation + localization (2026-06-30)

Re-verified after the rung-2 work landed (property-object cache 5.21, enum-format 5.22/5.23). The VM has
changed substantially since the spike, but **rung 3 fails identically**:

```bash
dotnet /tmp/ef-probe-out/EfApp.dll                   # Hello, exit 42
./target/debug/dotnet-rs /tmp/ef-probe-out/EfApp.dll # Internal VM error: ... Generic index 0 out of bounds (length 0), exit 1
```

**Localization (for whoever picks up the EF epic).** The error is raised in
`crates/dotnet-types/src/generics.rs` (`MethodType::TypeGeneric` ~440 / `MethodType::MethodGeneric` ~473)
from `GenericLookup::make_concrete`. `index 0, length 0` means a generic-parameter slot (`!0`/`!!0`) is
resolved against a `GenericLookup` whose **both** `type_generics` and `method_generics` are empty.

Per the trace above, the failing instantiation is the **generic static method**
`System.Collections.Immutable.ImmutableSortedDictionary.Create<TKey,TValue>(IComparer<TKey>)` with
`TKey=System.Type`, `TValue=System.ValueTuple<IDbContextOptionsExtension,int>`, reached from
`DbContextOptions..ctor`. The method-generic arguments are not reaching the resolution context where
`!!0`/`!!1` are looked up (the lookup carries zero args), so the `!!0` lookup underflows.

### Exact empty-lookup occurrence (Step 1.3 instrumentation, 2026-06-30)

Running host mode without `DOTNET_RS_TRACE`:

```bash
./target/debug/dotnet-rs /tmp/ef-probe-out/EfApp.dll
```

emits:

```text
[GENERIC-OOB] make_concrete caller=ResolutionS(System.Private.CoreLib @ 0x...) requested=TypeGeneric(0) lookup={}
Internal VM error: Type resolution failed: Generic index 0 out of bounds (length 0)
```

Stack context for this exact occurrence (from the rung-3 trace chain) is:

```text
BlogContext::.ctor
  -> DbContext::.ctor
    -> DbContextOptions`1<BlogContext>::.ctor
      -> DbContextOptions::.ctor
        -> call ImmutableSortedDictionary.Create<System.Type, ValueTuple<IDbContextOptionsExtension,int>>(...)
          -> ImmutableSortedDictionary.Create<TKey,TValue>
            -> IL_0000: ldsfld ImmutableSortedDictionary`2<!!TKey,!!TValue>::Empty
               -> GenericLookup::make_concrete(TypeGeneric(0), lookup={})  // underflow
```

This pins the blocker to an **empty `GenericLookup` at `make_concrete`** while resolving
`ImmutableSortedDictionary`2<!!0,!!1>::Empty` on the EF startup path.

This is the **same class** as the 5.4 constrained-callvirt bug (method generics not propagated to the
dispatched context). Start at `crates/dotnet-runtime-resolver/src/methods.rs::find_generic_method`
(~52–76, where `new_lookup.method_generics = params.into()`) and trace whether the dispatched frame's
lookup carries `method_generics` into the type-resolution calls executed inside the method body /
`.cctor` reached from it. **Note:** `generics.rs` already accretes heuristic index-probing fallbacks
(`lookup_method_generic_with_fallback_indices`, the type/method index-shift probes at ~436/453/461) —
the real fix is upstream propagation, **not** another fallback heuristic.

**Scope reality:** unlike rung-2 (a contained reflection-completeness gap that one cache fix unblocked),
this is a core generic type/method substitution bug, and EF is a large framework — clearing this blocker
will surface further ones. Treat rung-3/EF as a **separate epic**, not a quick follow-on to rung-2.

## EF epic handoff after supervised/ef-core finalization (2026-07-02)

The `supervised/ef-core` branch advanced the host-mode EF probe through the original generic
lookup failure and many downstream runtime/reflection/delegate/string blockers, but rung 3 is
still **FAIL** and should not be marked complete.

Final verification commands:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --workspace
cargo build -p dotnet-cli --bin dotnet-rs --no-default-features
bash scripts/diff_run.sh --host /tmp/ef-probe-out/EfApp.dll
```

Observed results:

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --all-targets --no-default-features -- -D warnings`: PASS.
- `cargo test --workspace`: PASS (requires `/tmp/fixture-probe` and `/tmp/nuget-probe-out`
  host-test artifacts to be present).
- `cargo build -p dotnet-cli --bin dotnet-rs --no-default-features`: PASS.
- Stock `dotnet /tmp/ef-probe-out/EfApp.dll`: prints `Hello`, exits `42`.
- Host-mode `dotnet-rs /tmp/ef-probe-out/EfApp.dll`: exits `1`.

Current next blocker:

```text
Unhandled Exception: System.NotImplementedException: Arg_NotImplementedException
Stack Trace:
   at System.Reflection.MemberInfo.GetCustomAttributesData() in IP 1
   at System.Reflection.NullabilityInfoContext.Create(System.Reflection.PropertyInfo propertyInfo) in IP 32
   at Microsoft.EntityFrameworkCore.Metadata.Conventions.NonNullableConventionBase.TryGetNullabilityInfo(...) in IP 38
   at Microsoft.EntityFrameworkCore.Metadata.Conventions.NonNullableReferencePropertyConvention.Process(...) in IP 11
   ...
   at Microsoft.EntityFrameworkCore.Internal.InternalDbSet`1.Add(T0 entity) in IP 2
   at Program.Main() in IP 10
```

This means the branch cleared the immediately prior missing-string-intrinsic blockers
(`StartsWith(string, StringComparison)` and `EndsWith(string, StringComparison)`) and now reaches
EF model nullability convention processing. The next refactor should start from
`System.Reflection.MemberInfo.GetCustomAttributesData()`/`NullabilityInfoContext` support rather
than continuing the old generic-lookup triage.

## EF epic handoff after supervised/ef-core-2 finalization (2026-07-04)

The `supervised/ef-core-2` branch advanced the host-mode EF probe past the
`GetCustomAttributesData`/`NullabilityInfoContext` wall and through a long
sequence of runtime, reflection, dispatch, string, array, and expression
interpreter blockers. Rung 3 is still **FAIL** and should not be marked
complete.

Current branch-local progress reached checklist step 2.31. The standard EF
probe still reports the outer EF query-translation failure, while the
diagnostic repro that exposes the inner materialization path now advances to:

```text
System.NullReferenceException: Object reference not set to an instance of an object.
   at Microsoft.EntityFrameworkCore.ChangeTracking.Internal.IdentityMap`1.TryGetEntry(...)
   at System.Linq.Expressions.Interpreter.ByRefMethodInfoCallInstruction.Run(...)
```

This means the branch cleared the immediately prior
`RuntimeHelpers.GetUninitializedObject(System.Type)` / `Serialization_InvalidType`
blocker and now reaches EF identity-map lookup during interpreted query
execution. The next EF refactor should start from this `IdentityMap<T>.TryGetEntry`
`NullReferenceException`, while preserving the standard rung-3 done condition:
stock `dotnet` and host-mode `dotnet-rs` must both print `Hello` and exit `42`.

Final verification commands:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features -- -D warnings
CARGO_BUILD_JOBS=1 bash check.sh
cargo build -p dotnet-cli --bin dotnet-rs --no-default-features
dotnet build tests/ef-probe/EfApp.csproj -c Debug
dotnet tests/ef-probe/bin/Debug/net10.0/EfApp.dll
./target/debug/dotnet-rs tests/ef-probe/bin/Debug/net10.0/EfApp.dll
./target/debug/dotnet-rs /tmp/ef-debug-probe/bin/Debug/net10.0/ef-debug-probe.dll
```

Observed results:

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --all-targets --no-default-features -- -D warnings`: PASS.
- `CARGO_BUILD_JOBS=1 bash check.sh`: PASS
  (`384/384`, `448/448`, and `466/466` test legs passed).
- `cargo build -p dotnet-cli --bin dotnet-rs --no-default-features`: PASS.
- `dotnet build tests/ef-probe/EfApp.csproj -c Debug`: PASS.
- Stock `dotnet tests/ef-probe/bin/Debug/net10.0/EfApp.dll`: prints `Hello`, exits `42`.
- Host-mode `dotnet-rs tests/ef-probe/bin/Debug/net10.0/EfApp.dll`: exits `1` with the outer
  `CoreStrings.TranslationFailed` query error for `DbSet<Blog>().First()`.
- Diagnostic host-mode `/tmp/ef-debug-probe`: exits `1`; the direct `Execute` path reaches
  `IdentityMap<T>.TryGetEntry(...)` `NullReferenceException`, then the normal `query.First()`
  path still reports the outer translation error.

## EF epic handoff after supervised/ef-core-3 finalization (2026-07-08)

The `supervised/ef-core-3` branch cleared both blockers named in the ef-core-2
handoff above: the inner `IdentityMap<T>.TryGetEntry` `NullReferenceException`
(materialization path) and the outer `DbSet<Blog>().First()`
`CoreStrings.TranslationFailed` query-translation failure. **Rung 3 (EF Core
InMemory) now PASSES its done condition:** stock `dotnet` and host-mode
`dotnet-rs` both print `Hello` and exit `42` for the real probe
`tests/ef-probe/EfApp.dll`.

Two root-cause fixes landed:

1. **By-ref value-type marshaling in `MethodInfo.Invoke`.**
   `unbox_param_to_stack_value` (`crates/dotnet-intrinsics-reflection/src/methods.rs`)
   now splits `ParameterType::Ref(t)` from `ParameterType::Value(t)`. For a
   value-type by-ref param it hands the callee a `ManagedPtr` into the boxed
   argument's interior (owner-pinned; a null/`out` arg gets a zero-initialized
   backing box that is written back into the `object[]`); for a reference-type
   by-ref param it hands a `ManagedPtr` into the `object[]` element slot. This
   fixed the `IdentityMap<T>.TryGetEntry(..., ref bool hasNullKey)` NRE at IP 44
   reached via `ByRefMethodInfoCallInstruction.Run → MethodBase.Invoke`.

2. **Method identity for definition-token lookups.**
   `AssemblyLoader::locate_method` (`crates/dotnet-assemblies/src/resolution.rs`)
   now keeps `parent_generics.method_generics` empty for `UserMethod::Definition`
   resolution. Method generic arguments belong to call-site lookup identity, not
   declaring-type identity; embedding them polluted `MethodDescription` identity
   and made EF's `GetGenericMethodDefinition() == QueryableMethods.FirstWithoutPredicate`
   comparison fail, throwing `TranslationFailed` in
   `NavigationExpandingExpressionVisitor.VisitMethodCall`.

Regression pins added under `crates/dotnet-cli/tests/fixtures/reflection/`:
`invoke_byref_42.cs`, `invoke_byref_writethrough_42.cs`, and
`reflection_queryable_first_method_identity_42.cs`.

Final verification commands:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features -- -D warnings
CARGO_BUILD_JOBS=1 bash check.sh
cargo build -p dotnet-cli --bin dotnet-rs --no-default-features
dotnet build tests/ef-probe/EfApp.csproj -c Debug
dotnet tests/ef-probe/bin/Debug/net10.0/EfApp.dll
./target/debug/dotnet-rs tests/ef-probe/bin/Debug/net10.0/EfApp.dll
```

Observed results:

- `cargo fmt --all -- --check`: PASS.
- `cargo clippy --all-targets --no-default-features -- -D warnings`: PASS.
- `CARGO_BUILD_JOBS=1 bash check.sh`: PASS (per Phase-5 gate; the finalizer added
  only a test-only `#[rustfmt::skip]` directive afterward, no functional change).
- `cargo build -p dotnet-cli --bin dotnet-rs --no-default-features`: PASS.
- `dotnet build tests/ef-probe/EfApp.csproj -c Debug`: PASS.
- Stock `dotnet tests/ef-probe/bin/Debug/net10.0/EfApp.dll`: prints `Hello`, exits `42`.
- Host-mode `dotnet-rs tests/ef-probe/bin/Debug/net10.0/EfApp.dll`: prints `Hello`, exits `42`.

**Scope reality:** the done condition covers the minimal InMemory probe
(`DbContext` + one `Blog` + `SaveChanges` + `First()`). It does not claim broad
EF query / relationship / migration / SQLite coverage — that remains the Option B
epic in `docs/USERLAND_TESTING_ROADMAP.md` §5, which adds filesystem + native
P/Invoke on top of what this rung validated.
