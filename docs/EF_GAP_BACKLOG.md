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
