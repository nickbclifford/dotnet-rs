# Review: Deeper End-to-End Testing of the Runtime via C#/.NET Userland

> Status: research / strategic review — **not** an implementation plan.
> Date: 2026-06-19. Author: research pass grounded in the current codebase + Microsoft Learn docs.
> Goal: identify high-leverage paths of continuation now that `dotnet-rs` is broadly featureful and
> optimized, with an emphasis on exercising the runtime through *real* C# userland rather than
> hand-written fixtures.

## 0. TL;DR / recommendations

The runtime is mature where it counts for this purpose: an IL interpreter that **executes real BCL/library IL directly** (intrinsics only where necessary), with full generics, complete exception handling, working read-side reflection *and* `MethodInfo.Invoke`. The gaps that gate userland are concentrated, not pervasive: **no dynamic-code policy, no real host/loader (deps.json/runtimeconfig/NuGet), thin I/O (no sockets/files), inline-only async, no Reflection.Emit.**

**Central insight:** the "real frameworks need `Reflection.Emit`" fear is largely false. The BCL ships a **non-emitting interpreter** for `Expression.Compile` that activates when the runtime reports `RuntimeFeature.IsDynamicCodeSupported == false`. Because `dotnet-rs` already runs IL-bodied BCL code, flipping that one switch routes EF/LINQ/DI hot paths onto interpreter code we can already execute. This reframes EF Core from "blocked" to "reachable."

**Why userland, not more fixtures:** a single EF query crosses expression trees, generics, reflection, attributes, collections, and async at once. Real libraries are pre-written, pre-correct IL with their own test suites — each framework is a forcing function that names a small, prioritized bundle of missing primitives and brings a ready-made oracle (differential execution vs `dotnet`; vendored test suites).

**Recommended sequence (cheap-and-reframing → expensive-and-narrow):**
1. **Flip the dynamic-code switch** (`IsDynamicCodeSupported=false`) — hours, unblocks the expression interpreter. *Highest information-per-effort in the document.*
2. **Language-conformance track + a differential harness** (Option E) — near-zero new substrate; builds the measurement infrastructure everything else uses.
3. **Runner over `dotnet`** (Option A) — read deps.json/runtimeconfig + roll-forward + NuGet-cache probing. The multiplier: turns every later target into `dotnet add package`.
4. **EF Core InMemory → SQLite** (Option B) — the headline engine-breadth target; InMemory is mostly steps 1+reflection, SQLite adds filesystem + native P/Invoke.
5. **Host xUnit**, then **sockets + async scheduler** only when a networked target demands them.
6. **Blazor *SSR* (render-to-string)** as a capstone (DI + renderer) — *not* Blazor Server/WASM. Razor is a build-time concern; the cost is the ASP.NET hosting substrate, so defer.
7. **Reflection.Emit stays reactive** — `DynamicMethod`-only (Tier 1) if a high-value target forces it; `TypeBuilder` (proxies/mocks) out of scope.

**First sitting:** probes 1, 2, and 8 in §10 — a tiny runtime change plus the differential oracle — jointly determine whether EF is reachable and give us the means to prove it.

## 1. Where the runtime stands today (grounding)

A capability snapshot, filtered to what matters for running *real* userland code.

**Execution & language core — strong.**
- Bytecode interpreter with build-time-generated jump-table dispatch (`crates/dotnet-vm/src/dispatch/mod.rs`, `crates/dotnet-vm/build_support/codegen.rs`). ~196 CIL instructions handled; unmatched ops fall through to `ExecutionError::NotImplemented` rather than `todo!`.
- Generics fully instantiated (types + methods, constraints, variance) with a thread-local lookup cache (`crates/dotnet-types/src/generics.rs`).
- Exception handling is complete ECMA-335 two-pass SEH: try/catch/finally/fault/filter + `leave` (`crates/dotnet-exceptions/src/lib.rs`).
- Delegates/multicast, boxing, arrays, value types, interfaces, iterators (`yield`), and LINQ-to-objects all pass in the fixture corpus (`crates/dotnet-cli/tests/fixtures/`).

**BCL strategy — hybrid, and this is the key enabler.** Methods with a CIL body are *interpreted directly* from the real shared-framework assemblies; only performance-critical or runtime-dependent APIs are replaced by Rust intrinsics (~444 handlers across `crates/dotnet-intrinsics-*` + `crates/dotnet-vm/src/intrinsics/`). A small embedded `support.dll` (`crates/dotnet-assemblies/src/support/*.cs`) supplies stand-in implementations for runtime-coupled types (Delegate, Array, Task, RuntimeType, MethodInfo, …) tagged with `[Stub]`. **Consequence: any userland or library code that is "just IL over types we already model" tends to run without new Rust work** — this is precisely why throwing real frameworks at it is cheap conformance pressure.

**Reflection — read path good, invoke works, emit absent.**
- Type/MethodInfo/FieldInfo/ParameterInfo introspection, `typeof`, `GetType`, generic reflection, `Activator.CreateInstance` are implemented (`crates/dotnet-intrinsics-reflection/`).
- `MethodInfo.Invoke` / `ConstructorInfo.Invoke` *are* implemented via an `awaiting_invoke_return` re-entrancy mechanism (`crates/dotnet-intrinsics-reflection/src/methods.rs:191-263`, `crates/dotnet-vm-data/src/stack.rs:56`, `crates/dotnet-vm/src/stack/context.rs:288`).
- **No `System.Reflection.Emit`, no `DynamicMethod`, no `ILGenerator`.** This is the single biggest userland blocker and the hinge for Sections 3.1, 5, and 7.

**Async — runs synchronously, no scheduler.** The embedded `AsyncTaskMethodBuilder` (`crates/dotnet-assemblies/src/support/AsyncMethodBuilders.cs`) drives the state machine inline on the calling thread: `Start` calls `MoveNext`, continuations are plain `Action`s. So `await` over already-completed / `TaskCompletionSource`-driven work passes (see `crates/dotnet-cli/tests/fixtures/async/`), but there is **no thread pool, no `TaskScheduler`, no timer** — `Task.Run`, `Task.Delay`, and genuine background concurrency have nothing to run on.

**Threading & GC — real but stop-the-world.** Feature-gated real OS threads with Monitor/Interlocked/Volatile and safe-point-coordinated STW GC (`crates/dotnet-vm/src/threading/basic.rs`, `crates/dotnet-vm/src/gc/coordinator.rs`). P/Invoke is functional via libffi + libloading (`crates/dotnet-pinvoke/`).

**I/O substrate — essentially absent.** No grep hits for `Socket`, `HttpClient`, `FileStream`, `System.Net`, `std::net`, or `std::fs::File` across the intrinsic/VM/pinvoke crates. No file-I/O fixtures. Console output works; everything else touching the OS does not.

**Loader/host — single flat directory only.** `--assemblies <FOLDER>` + one entry `.dll` (`crates/dotnet-cli/src/lib.rs:18-29`). The runtime is located by scanning `DOTNET_ROOT`/well-known paths for the latest `Microsoft.NETCore.App` (`crates/dotnet-assemblies/src/resolution.rs:881-906`). **No `deps.json`, no `runtimeconfig.json`, no NuGet, no hostfxr** — there is an optional manual `redirects.txt` only. This is the gap Option A closes.

**Test harness.** Fixtures are `{feature}_{exitcode}.cs` compiled on demand via `dotnet build` of a single-file csproj, then run in-process and checked by exit code (`crates/dotnet-cli/build.rs`, `crates/dotnet-cli/tests/integration_tests_impl/harness.rs`). ~150 fixtures across 22 categories; 15 benchmark workloads (`crates/dotnet-benchmarks/`). The corpus is hand-authored and small relative to the runtime's actual surface.

## 2. The testing thesis: why userland frameworks are the right pressure

The current corpus is ~150 hand-authored fixtures, each crafted to isolate one feature and assert one exit code. That is excellent for *regression localization* but has three structural weaknesses as the runtime matures:

1. **It tests what we already thought to test.** Hand-written fixtures encode the author's model of the runtime. Real frameworks encode *Microsoft's* model — they exercise opcode/BCL/edge-case combinations no one on this project would think to write (e.g. a single EF query touches expression trees, generics, reflection, `IEnumerable` plumbing, string formatting, and dictionary internals in one shot).
2. **Coverage scales linearly with author effort.** A 2,000-line fixture is 2,000 lines someone wrote. A NuGet package is tens of thousands of lines of *already-written, already-correct, already-shipping* IL whose own test suite can become our oracle.
3. **It doesn't pressure the seams.** The interesting bugs in a young runtime are rarely in one opcode; they are at the *boundaries* — generic-over-generic instantiation, exceptions thrown through reflection-invoked frames, a static cctor that runs during type resolution during a GC safe point. Userland code crosses these seams constantly and incidentally.

**The leverage comes from the hybrid BCL model (§1).** Because IL-bodied methods execute directly, a large fraction of any managed library "just runs" once its *transitive runtime dependencies* are modeled. So the marginal cost of a new framework is not "reimplement the framework" — it is "fill whatever small set of runtime primitives that framework bottoms out on." Each framework is therefore a **forcing function that names a coherent, prioritized bundle of missing primitives** rather than a vague backlog.

This reframes the whole document: the options below are not "features to build" — they are **conformance targets that each pull a specific substrate into existence**, with a ready-made external test suite attached. The right question for each is: *what is the smallest substrate that makes this framework's own tests run, and what does getting there incidentally prove about the runtime?*

Two practical oracles fall out of this:
- **Differential execution.** Run the same assembly under `dotnet` and under `dotnet-rs`; diff exit code, stdout, and (later) emitted SQL / rendered HTML. Any divergence is a bug with a known input.
- **Vendored test suites.** Many of these libraries ship xUnit/NUnit suites. Once the runtime can host a test runner (itself a worthwhile target — see §8), their green/red becomes our green/red for free.

## 3. Cross-cutting capability gaps (the shared substrate)

Before the options, the substrate. Almost every userland target below bottoms out on the same handful of primitives. Building these *first* means each framework target becomes incremental rather than a from-scratch effort, and several of them are independently valuable (and independently testable).

### 3.1 Dynamic code policy (`IsDynamicCodeSupported`) and the expression interpreter

**Status (2026-06-20, Phases 1–2 + finalizer): ✅ Completed.** `AppContext.TryGetSwitch` + seeded `IsDynamicCodeSupported=false` are in place; `basic/runtime_feature_dynamic_code_42.cs` verifies both `RuntimeFeature.IsDynamicCodeSupported` and `IsDynamicCodeCompiled` read false, and `expressions/expression_compile_42.cs` (`Expression.Compile()(41)`) now passes with exit code 42.

**This is the most important single finding in this review.** The naive read is "EF / DI / serializers need `Reflection.Emit`, and we have none, so they're out." That is wrong, because the BCL itself ships a *non-emitting* fallback.

`System.Linq.Expressions.LambdaExpression.Compile` has an overload `Compile(bool preferInterpretation)`, and the whole expression stack consults `RuntimeFeature.IsDynamicCodeSupported`. On platforms that forbid runtime code generation (iOS full-AOT, NativeAOT, wasm), `Compile()` does **not** emit IL via `DynamicMethod` — it returns a delegate backed by an **interpreter written in ordinary IL** (the "light compiler" / interpreter in `System.Linq.Expressions`). That interpreter is exactly the kind of IL-bodied BCL code `dotnet-rs` already executes natively.

So the lever is: **make the runtime advertise `IsDynamicCodeSupported == false`** (and `IsDynamicCodeCompiled == false`). Today the runtime handles neither — grep for `IsDynamicCodeSupported`/`RuntimeFeature` returns nothing. `RuntimeFeature` is just a property reading an `AppContext` switch (`RuntimeFeature.NonNativeAot.cs`), so this is a small, well-scoped intrinsic/feature-switch addition, not a subsystem.

Once that switch reads `false`, large swaths of the modern BCL silently route onto interpreter/reflection paths instead of emit paths:
- `Expression<T>.Compile()` → interpreter delegate (unlocks LINQ provider plumbing, EF's parameter binders, lots of DI/serializer hot paths).
- Many `[RequiresDynamicCode]`-guarded APIs have a feature-guarded fallback (the `[FeatureGuard(typeof(RequiresDynamicCodeAttribute))]` pattern, .NET 9+).

**Caveats / what this does *not* free:** `Type.MakeGenericType` over a runtime-only type combination is `[RequiresDynamicCode]` because the *runtime* (not Roslyn) must materialize the instantiation — but `dotnet-rs` already instantiates generics dynamically (§1), so this is a non-issue for us specifically. The real residue is code that *unconditionally* calls `DynamicMethod`/`ILGenerator` with no interpreter fallback (some high-perf serializers, Castle DynamicProxy used by EF lazy-loading proxies and most mocking libraries). Those need Option D or are simply declared out of scope.

**Probe:** set the switch, run a fixture that does `Expression<Func<int,int>> f = x => x + 1; return f.Compile()(41);` expecting `42`. If that runs, the interpreter path works and EF moves from "blocked" to "substrate-bounded."

### 3.2 Assembly/host resolution: deps.json, runtimeconfig.json, NuGet

The runtime today takes one flat directory of DLLs. Real apps are described by metadata the official host reads. Implementing a faithful-enough subset is the entirety of Option A and a prerequisite for loading anything with NuGet dependencies.

- **`*.runtimeconfig.json`** — names the target framework, the framework reference(s) (`Microsoft.NETCore.App`, and for web `Microsoft.AspNetCore.App`), the `rollForward` policy, and `configProperties` (which includes feature switches like the one in §3.1). The shared-framework *version* is chosen by roll-forward rules: default `Minor` = lowest acceptable minor ≥ requested, then highest patch; other policies `LatestPatch`/`Major`/`LatestMinor`/`LatestMajor`/`Disable`. We already pick "latest installed" crudely; this replaces that with the real algorithm.
- **`*.deps.json`** — the dependency manifest. It lists `targets` → `libraries` with `runtime` (managed), `native`, and `resources` assets, plus rid-specific asset groups. The host turns it into the four probing properties consumed by `AssemblyLoadContext.Default`: `TRUSTED_PLATFORM_ASSEMBLIES` (managed assemblies), `NATIVE_DLL_SEARCH_DIRECTORIES`, `PLATFORM_RESOURCE_ROOTS`, `APP_PATHS`. RID asset selection in modern .NET no longer walks the RID graph — it checks a fixed portable-RID fallback list (e.g. `linux-x64 → linux → unix-x64 → unix → any`).
- **NuGet** — `dotnet restore` resolves `PackageReference`s into a dependency graph and writes `obj/project.assets.json`, with packages unpacked under the global packages folder (`~/.nuget/packages`). A *runner* doesn't need to re-implement restore: it can shell out to `dotnet restore`/`dotnet build` (or read an already-restored `project.assets.json` + the produced `deps.json`) and then load from `~/.nuget/packages` using the deps.json asset lists.

Net: a `deps.json` + `runtimeconfig.json` reader plus roll-forward gives the runtime a real, multi-assembly, NuGet-aware loader — and turns "compile a fixture" into "point at any published app."

### 3.3 I/O substrate: files, sockets, time, threads/scheduler

This is the widest gap (§1: no networking/file intrinsics) and it gates the server-shaped options (ASP.NET/Blazor Server, EF over a real DB). It decomposes into independent layers, cheapest first:

- **Time & timers.** `DateTime.UtcNow`, `Stopwatch`, `Environment.TickCount`, and a timer queue. Cheap, pure-host, and a prerequisite for `Task.Delay`, cache expiry, logging timestamps. No external surface.
- **Filesystem.** `FileStream`/`File`/`Directory` over `std::fs`. Self-contained, easy to differential-test, and immediately enables file-backed test corpora, config loading, and EF-SQLite's database file. Medium effort, low risk.
- **Sockets / networking.** `System.Net.Sockets.Socket` over `std::net` / mio. This is the real cost center and it's what Kestrel/HttpClient/DB-wire-protocols need. It also drags in the async story (§3.5) because socket readiness is the canonical thing you `await`. High effort.
- **Process/env.** `Environment`, `Process` — mostly trivial, partly already present (Console).

The sequencing insight: **time + filesystem are cheap and unlock a lot (EF InMemory/SQLite, config, logging); sockets are expensive and should be deferred until a socket-shaped target is actually the goal.**

### 3.4 Reflection depth & Reflection.Emit

Read-side reflection and `Invoke` already work (§1). The two missing tiers:

- **Custom attributes & richer member queries at runtime.** Some frameworks read attributes reflectively (EF conventions, DI `[Inject]`, serializer `[JsonPropertyName]`, xUnit `[Fact]`). Verify and extend `GetCustomAttributes`, attribute instantiation, property/event enumeration, and `MemberInfo` coverage. This is "more of what we have," not a new subsystem — and it's exercised heavily by every framework below, so it improves naturally under userland pressure.
- **`System.Reflection.Emit` / `DynamicMethod` / `ILGenerator` / `TypeBuilder`.** Genuinely large: it means accepting emitted IL into the same execution pipeline as loaded IL (a `DynamicMethod` is "a method with a body assembled at runtime"). Because `dotnet-rs` is an *interpreter*, this is more tractable than for a JIT — you don't compile the emitted IL, you just interpret it — but `TypeBuilder` (whole emitted types with metadata) is a deep rabbit hole. **Recommendation: lean on §3.1 to avoid this for as long as possible**; pursue `DynamicMethod`-only emit (no `TypeBuilder`) as a bounded Option D if a high-value target truly requires it (e.g. a serializer with no interpreter fallback). Castle DynamicProxy (EF lazy proxies, Moq) needs `TypeBuilder` and should be treated as out of scope.

### 3.5 Real async (scheduler / timers / thread pool)

Today async completes inline (§1). To run server frameworks or anything that awaits I/O, the runtime needs:

- **A `ThreadPool`** to back `Task.Run` and continuation dispatch.
- **A `TaskScheduler` / `SynchronizationContext`** so continuations land somewhere real.
- **A timer queue** for `Task.Delay`/`CancellationTokenSource(timeout)` (overlaps §3.3 time).
- Eventually, **I/O completion integration** so an `await socket.ReadAsync(...)` suspends and resumes on readiness rather than spinning a thread.

The existing `AsyncTaskMethodBuilder` stub is the right seam: it already routes completion through `Action` continuations, so swapping "run inline" for "queue on a scheduler" is localized. **Effort is moderate for the pool+scheduler+timer (unlocks `Task.Run`, `Task.Delay`, `Task.WhenAll` with real concurrency); high once it must integrate with async sockets.** Sequence this with §3.3: do pool+timer when a CPU-bound-concurrency target appears; do I/O integration only alongside sockets.

## 4. Option A — `dotnet-rs` as a runner over `dotnet` (SDK + NuGet host)

**Thesis: this is the highest-leverage option and the natural first move, because it is the multiplier for every other option.** It does not require new execution-engine capability — it is a *loader/host* project (§3.2) that converts "I hand-built a single DLL" into "point `dotnet-rs` at any normal C# project or its publish output and it runs." Every framework target afterward is then just `dotnet add package X` away from being a test case.

**Why it's tractable.** We can stand on the official SDK rather than reimplement it. The split:
- Let `dotnet` (the SDK) do `restore` + `build`/`publish`. That produces, next to the app DLL: `app.dll`, `app.runtimeconfig.json`, `app.deps.json`, and a populated `~/.nuget/packages`. These artifacts are a *complete, machine-readable description* of what to load.
- `dotnet-rs` becomes the **host**: parse `runtimeconfig.json` → resolve the shared framework via roll-forward; parse `deps.json` → build the assembly + native-library probing lists (`TRUSTED_PLATFORM_ASSEMBLIES`, `NATIVE_DLL_SEARCH_DIRECTORIES`); load and run.

This replaces today's "scan one folder, pick latest runtime" (`crates/dotnet-assemblies/src/resolution.rs:881-906`) with the real resolution algorithm, and generalizes the loader from one flat directory to the app-base + shared-framework(s) + NuGet-cache probing model.

**Phased milestones (each independently shippable and testable):**
1. **`runtimeconfig.json` reader + roll-forward.** Pick the shared-framework version the way the host does. Test: run today's fixtures via their real `runtimeconfig.json` instead of the `--assemblies` flag.
2. **`deps.json` reader + multi-path probing.** Load managed assemblies from app-base + framework dir + `~/.nuget/packages` per the deps.json asset lists. Test: a project with one pure-managed `PackageReference` (e.g. `Newtonsoft.Json`) runs.
3. **`dotnet-rs run <project-or-dir>` UX.** Detect a `.csproj`/published folder; if a project, shell out to `dotnet build` then host the output (mirror `dotnet run` = build-then-exec). Keep the existing low-level `--assemblies` mode for the test harness.
4. **Native-library probing** wired to the existing P/Invoke loader (`crates/dotnet-pinvoke/`), so packages carrying native assets (later: SQLite) resolve.
5. **`configProperties` → feature switches**, including §3.1's `IsDynamicCodeSupported` and `System.Runtime.Loader.UseRidGraph`-style toggles, read from `runtimeconfig.json` and surfaced via `AppContext`.

**Out of scope / cannot run** (worth stating explicitly in the runner so it fails loudly, not weirdly):
- **NativeAOT publish** → a native executable with no IL; nothing for an interpreter to load.
- **Single-file bundles** → require bundle extraction before the DLLs are visible (could be supported later by reusing the host's extraction logic, but defer).
- **ReadyToRun (R2R)** images → *fine*: they still carry IL, and an interpreter just ignores the precompiled native code and interprets the IL.

**Payoff for testing.** This is the option that changes the *kind* of testing available: from "author a fixture" to "vendor a real app or its test suite and diff against `dotnet`." It also produces a crisp, public capability story ("`dotnet-rs run ./MyApp`"). Recommended as the first major investment; §3.2 is its body of work.

## 5. Option B — Entity Framework Core

**Thesis: EF Core is the single best *execution-engine* stress target in the ecosystem, and §3.1 makes it plausible without `Reflection.Emit`.** One EF query exercises expression trees, deep generics, runtime reflection, custom attributes, `IEnumerable`/`IQueryable` plumbing, string/format machinery, dictionaries, and async — simultaneously. If EF's own functional test suite goes green, an enormous amount of the runtime has been transitively validated by code we didn't write.

**What EF actually requires from a runtime:**
- **Expression trees + `Compile`** — EF builds `Expression` trees from your LINQ, compiles a query into SQL once, caches by tree shape, and compiles *materializer/parameter-binder delegates* via `Expression.Compile`. With §3.1's switch set to `false`, those compile to **interpreter delegates** (pure IL) rather than emitted IL. This is the make-or-break dependency and the reason EF is feasible at all here.
- **Heavy reflection + custom attributes** — model building reads CLR types, properties, and conventions/attributes reflectively. Leans on §3.4's read-side reflection (already mostly present) plus attribute coverage.
- **Async** — the public API is `ToListAsync`/`SaveChangesAsync`/etc. The **InMemory** and SQLite providers complete most operations synchronously, so EF runs under the current inline-async model (§1) for a large fraction of operations; real async DB I/O would need §3.5 + sockets, but that's only the networked-provider case.
- **A provider** — EF needs a database provider, and *this is the choice that sets the cost*:
  - **`Microsoft.EntityFrameworkCore.InMemory`** — pure managed, no native, no SQL, no I/O. **This is the recommended first target:** it isolates EF's core machinery (LINQ → expression compile → materialization → change tracking) from any I/O substrate. It is the cheapest possible way to put EF's engine under load.
  - **`Microsoft.EntityFrameworkCore.Sqlite`** — adds a real relational provider (SQL generation, the relational query pipeline) but pulls in `SQLitePCLRaw` → **P/Invoke to native `sqlite3`** and a database **file** (so it needs §3.3 filesystem + the §4 native-library probing, but *not* sockets). A strong second target because it exercises the relational pipeline and our P/Invoke + native-asset loading end-to-end.
  - **SQL Server / Npgsql** — wire-protocol over sockets → needs §3.3 sockets + §3.5 async I/O. Defer.

**What EF will *not* get for free:**
- **Lazy-loading proxies** (`UseLazyLoadingProxies`) use Castle DynamicProxy → `TypeBuilder` (§3.4). Out of scope; EF works fine without them (explicit/eager loading).
- **Compiled queries** (`EF.CompileQuery`) are an optimization, not a requirement; the normal cached path suffices.

**Phased milestones:**
1. **Substrate:** §3.1 switch + verify §3.4 reflection/attributes. Probe: `Expression.Compile` interpreter smoke test (§3.1) and a reflective attribute read.
2. **EF InMemory minimal:** define a `DbContext` with one entity, `Add` + `SaveChanges` + a `Where/Select/ToList` query, assert results. This is the headline milestone — EF's core engine running on `dotnet-rs`.
3. **EF InMemory breadth:** joins, grouping, navigation properties, change tracking, then vendor a slice of EF's InMemory functional tests as the oracle.
4. **EF SQLite:** filesystem (§3.3) + native `sqlite3` via §4 probing; assert generated SQL and round-tripped data match `dotnet`.

**Payoff for testing.** Best-in-class breadth-per-effort for the *interpreter and BCL*. The InMemory milestone in particular is mostly a §3.1 + §3.4 exercise — it may be reachable surprisingly early and would be a dramatic demonstration. This is the recommended first *framework* target once Option A's loader (or at least a hand-assembled package folder) exists.

## 6. Option C — Razor / Blazor

**Thesis: the scary part of Razor/Blazor is a *build-time* concern that `dotnet` already handles, but the runtime surface (ASP.NET Core hosting) is the most expensive substrate in this document. Pick the variant deliberately.**

**The crucial split — Razor compiles at build time.** A `.razor`/`.cshtml` file is turned into a normal C# class by the **Razor source generator** during `dotnet build`; "Blazor doesn't support runtime compilation of the UI logic" (MS docs). So *there is no runtime templating engine to implement* — the output is ordinary IL assemblies that the interpreter runs like any other. (The exception is MVC's optional *runtime* Razor compilation, which invokes Roslyn at runtime — explicitly out of scope.) This means Razor/Blazor adds **zero new execution-engine requirements**; the entire cost is in the *hosting runtime* each variant needs.

**The three variants, by runtime cost:**

- **Blazor WebAssembly** — designed to run on the in-browser wasm .NET runtime. `dotnet-rs` is itself a runtime; running someone's Blazor WASM app server-side on `dotnet-rs` is a category error. **Not a meaningful target.** (One narrow, interesting angle: the wasm runtime is itself "an IL interpreter with partial JIT" — conceptually a cousin of this project — but that's a comparison, not a target.)

- **Blazor Server** — Razor components run on the server; the browser is a thin client over a **SignalR** (WebSocket) connection. To host it, `dotnet-rs` needs essentially all of **ASP.NET Core**: Kestrel (sockets + TLS), the hosting/DI stack, SignalR, the renderer. That means §3.3 **sockets**, §3.5 **real async I/O**, dependency injection, and a large `Microsoft.AspNetCore.App` framework surface. This is the **most expensive substrate in this review** — it transitively requires almost everything. High payoff as a capstone, but not an early target.

- **Static server-side rendering (SSR) / `RazorComponent` "render to string"** — render a Razor component tree to an HTML string with no live connection, no WebSocket, no client interactivity. This isolates the **component-rendering engine** (DI + the renderer + the build-time-compiled component IL) from the **network/transport** layer. **This is the realistic Blazor-shaped target:** it exercises a big, real framework (DI container, component lifecycle, rendering) while needing far less of §3.3/§3.5 than Blazor Server — primarily DI and the renderer, plus whatever minimal hosting the render entry point pulls in.

**Recommended framing.** Treat "render a Razor component to HTML and diff against `dotnet`" as the Blazor objective, not "host a live Blazor Server app." It is the part that pressures the *managed* framework engine (the interesting part for this runtime) without paying for the entire network stack. The DI container (`Microsoft.Extensions.DependencyInjection`) that this requires is itself a valuable, broadly-used substrate — it leans on reflection + activation (§3.4) and benefits many other targets.

**Payoff for testing.** Lower breadth-per-effort than EF until the ASP.NET surface exists, *but* the SSR slice doubles as the forcing function for the DI container and the hosting-extensions stack, which are dependencies of essentially the entire modern .NET server ecosystem. Sequence it **after** EF (which validates the engine more cheaply) and **after** Option A (so the large `Microsoft.AspNetCore.App` framework set loads via deps.json rather than by hand).

## 7. Option D — Deeper reflection & Reflection.Emit / DynamicMethod

**Thesis: pursue this *reactively*, not proactively. §3.1 is designed specifically to avoid needing it, and an interpreter makes the cheap tier (`DynamicMethod`) far more tractable than the expensive tier (`TypeBuilder`) — but the expensive tier is a genuine rabbit hole. Build only the tier a concrete, high-value target forces.**

The opportunity is real because **`dotnet-rs` is an interpreter, not a JIT.** For a JIT, "emit IL at runtime" means a second compiler backend. For an interpreter, a `DynamicMethod` is simply *a method whose body arrived at runtime instead of from a loaded module* — and the runtime already has an execution loop that walks method bodies. The work is plumbing emitted IL + a synthesized method descriptor into the same pipeline (`crates/dotnet-vm/src/dispatch/`), reusing the existing interpreter wholesale.

**Two tiers, very different costs:**

- **Tier 1 — `DynamicMethod` + `ILGenerator` (bounded).** Accept an IL stream + signature, wrap it as an invokable/`Delegate`-backed method, interpret it. No new metadata tables, no new types. This unlocks the class of libraries that emit a *standalone method* (fast property getters/setters, some serializers' hot paths, `Expression.Compile`'s emit path if we ever wanted the compiled — not interpreted — variant). Medium effort; mostly "synthesize a method body and feed the existing loop."
- **Tier 2 — `TypeBuilder`/`AssemblyBuilder` (deep).** Define whole new types with fields, methods, and metadata at runtime, then instantiate them. This means a runtime metadata-emission layer interoperating with the loaded-metadata model in `crates/dotnet-assemblies` and `crates/dotnet-types`. This is what **Castle DynamicProxy** (EF lazy-loading proxies, Moq, many AOP/mocking libraries) needs. **High effort, high risk; recommend out of scope** unless a must-have target has no alternative.

**Why not just build it for completeness:** every hour here is an hour not spent on substrate that §3.1 already routed around. The honest cost/benefit is that the *interpreter fallback* (§3.1) covers the high-value cases (LINQ/EF/expression compilation) at a fraction of the cost, and the residual emit-only consumers (proxy/mock libraries) are mostly *test-infrastructure* libraries we can avoid by choosing frameworks that don't require them.

**Concrete trigger conditions** (when Tier 1 becomes worth it):
- A target framework's core path calls `DynamicMethod` *without* an interpreter fallback and the framework is otherwise high value.
- We want to host a mainstream **serializer** (System.Text.Json's source-generated path needs none of this; its reflection path may, depending on config) at full fidelity.

**Payoff for testing.** Narrow but deep: Tier 1 unlocks a specific, identifiable set of libraries; Tier 2 unlocks proxy/mock-based test suites. Neither is a *breadth* play. Keep this option as a documented escape hatch with explicit trigger conditions rather than a planned milestone.

## 8. Option E — C#-specific language-feature conformance

**Thesis: the cheapest, lowest-risk way to expand coverage *right now* — it needs essentially no new substrate, just more (and more cleverly sourced) IL fed through the existing harness. It pairs well as the "always-on" background track beneath the bigger options.**

The runtime already passes a broad slice of C# (generics, iterators, LINQ, delegates, async-inline, structs, unsafe). The gap is **systematic** coverage of language features as the compiler actually lowers them — and modern C# lowering is where interpreter bugs hide, because the compiler emits IL patterns hand-written fixtures rarely reproduce.

**High-value language surfaces to target deliberately** (each is "just IL" — no new runtime subsystem, only possibly small intrinsic gaps):
- **`async`/`await` lowering edge cases** that the current inline model mishandles — async iterators (`IAsyncEnumerable`/`await foreach`), `ConfigureAwait`, multiple suspension points, exceptions across awaits. (Some of these will *expose* the §3.5 scheduler gap — that's the point: they tell us precisely what's missing.)
- **Pattern matching** — switch expressions, property/positional/list patterns, relational and `and`/`or` patterns. Heavy, branch-dense lowering.
- **Records & `with` expressions** — synthesized equality, `Deconstruct`, `<Clone>$`, init-only setters.
- **Closures/captures** — display classes, captured loop variables, nested lambdas (interacts with GC).
- **`ref` features** — `ref struct`, `ref` returns/locals, `scoped`, `Span<T>` ergonomics (partly covered).
- **`default` interface methods, static abstract interface members** (generic math) — newer dispatch shapes.
- **Tuples, `stackalloc`, collection expressions (`[..]`), required members, primary constructors.**

**Sourcing the corpus — the real opportunity is to stop hand-writing it:**
1. **Vendor the Roslyn / runtime language test suites.** The dotnet/runtime and dotnet/roslyn repos contain large language-conformance corpora. Running even a subset under the differential oracle (run under `dotnet`, run under `dotnet-rs`, diff) converts Microsoft's test investment into ours.
2. **Source-generator output as a fixture mine.** Generators (records, regex `[GeneratedRegex]`, `System.Text.Json` source gen, LINQ) emit large, idiomatic IL at build time. Compiling small programs that *use* generators yields realistic IL bundles for free.
3. **Host a real test runner (gateway capability).** Getting **xUnit** to run on `dotnet-rs` is itself an Option-E milestone (xUnit is mostly managed; its discovery uses reflection/attributes from §3.4) — and once it runs, *every* vendored `[Fact]`-based suite (including EF's) reports green/red natively, no bespoke harness per target.

**Payoff for testing.** The best effort-to-coverage ratio for the *language/interpreter* layer, and it compounds: the differential harness + a hosted test runner built here are the measurement infrastructure that makes Options A/B/C measurable. Run it continuously, not as a one-shot.

## 9. Comparison & sequencing

Scoring is relative and deliberately coarse. "Testing payoff" = how much of the runtime is transitively validated per unit effort. "New substrate" = how much §3 work it forces.

| Option | Testing payoff | Effort | Risk | New substrate forced | Depends on |
|---|---|---|---|---|---|
| **E — Language conformance** | High (interpreter/lang layer) | **Low** | Low | ~none (maybe small intrinsics) | existing harness |
| **A — Runner over `dotnet`** | High (multiplier for all) | Medium | Low–Med | §3.2 (loader/host) | — |
| **B — EF Core (InMemory → SQLite)** | **Very high** (engine breadth) | Med (InMemory) → Med-High (SQLite) | Medium | §3.1, §3.4; then §3.3 fs + native | §3.1; eases with A |
| **C — Razor/Blazor SSR** | High (DI + render engine) | High | Med-High | DI, renderer; (Server: §3.3 sockets + §3.5) | A; after B |
| **D — Reflection.Emit** | Narrow/deep | Med (Tier 1) → High (Tier 2) | High (Tier 2) | emit→interpreter plumbing | reactive only |

**The dependency spine.** §3.1 (dynamic-code switch) is a tiny change that unblocks B and de-risks the whole "frameworks need emit" worry — **do it first, regardless.** §3.2 *is* Option A and is the multiplier for B and C. §3.3/§3.5 (I/O + async scheduler) are the expensive substrate that only the server-shaped tail (EF-networked, Blazor Server) truly needs — defer until a target demands them.

**Recommended order:**
1. **§3.1 dynamic-code switch** + its expression-compile probe. Hours-to-days; immediately reframes feasibility.
2. **Option E, as a continuous track** — vendor a language-conformance slice + stand up the differential oracle. This builds the *measurement* infrastructure the rest relies on.
3. **Option A (the runner)** — §3.2 milestones 1→5. Converts every later target into "add a package."
4. **Option B / EF InMemory** — the headline framework milestone; mostly §3.1 + §3.4. Then **EF SQLite** (adds §3.3 filesystem + native probing).
5. **Host xUnit** (an Option E milestone) so EF's own suite reports natively.
6. **§3.3 sockets + §3.5 async I/O** — only now, driven by a concrete networked target.
7. **Option C / Blazor SSR** as a capstone (DI + renderer); Blazor Server only if the full ASP.NET surface becomes a goal.
8. **Option D** stays reactive — built only when a must-have target trips a trigger condition (§7).

**One-line rationale:** start with the change that costs least and reframes the most (§3.1), build the measurement and loading infrastructure (E + A), then let EF pull the engine through its paces before paying for the network stack.

## 10. Concrete next experiments (cheap probes)

Each is a few hours to a few days, produces a clear yes/no, and de-risks a major option before committing to it. Most are new fixtures under `crates/dotnet-cli/tests/fixtures/` plus, where noted, a small runtime change.

1. **Dynamic-code switch + expression interpreter (de-risks B).** Add `RuntimeFeature.IsDynamicCodeSupported => false` (intrinsic or `AppContext` switch). Fixture: `Expression<Func<int,int>> f = x => x + 1; return f.Compile()(41);` expecting 42. *If green:* the BCL's expression interpreter runs on us → EF is substrate-bounded, not blocked. *This is the highest-information probe in the document.* **Status (2026-06-20, Phase 2 + finalizer): ✅ Completed.** Switch seeding + `AppContext` intrinsics landed; `runtime_feature_dynamic_code_42` directly verifies both dynamic-code RuntimeFeature properties are false, and `expression_compile_42` now returns 42.
2. **Reflection-invoke + attribute read (de-risks B/C/E).** Fixture that reads a custom attribute off a type and `MethodInfo.Invoke`s a method discovered reflectively. Confirms the §3.4 read path at the fidelity frameworks assume. **Status (2026-06-20, Phase 2): ✅ Completed.** `reflection_custom_attr_42` and `reflection_method_via_attr_42` both pass, proving attribute materialization/discovery and reflective invoke via attribute-selected method.
3. **`deps.json`/`runtimeconfig.json` parse-and-run (de-risks A).** Don't build the full host yet — write a throwaway: parse a real fixture's `*.deps.json` + `*.runtimeconfig.json`, derive the assembly list, feed the existing loader. *If it runs an existing fixture this way:* Option A milestone 1–2 is validated end-to-end.
4. **One pure-managed NuGet package (de-risks A).** Build a trivial app referencing `Newtonsoft.Json`, restore, then load `Newtonsoft.Json.dll` from `~/.nuget/packages` and round-trip a small object. Confirms multi-assembly NuGet loading with zero native deps.
5. **EF InMemory smoke (de-risks B, the headline).** Minimal `DbContext`, one entity, `Add`+`SaveChanges`+`Where/ToList`. Run under `dotnet` and `dotnet-rs`; diff. Gated on probes 1–2. *This is the milestone that would prove the thesis.*
6. **`DateTime.UtcNow` + `Task.Delay(0)` (de-risks §3.3/§3.5 scope).** Tiny time intrinsic + see how far inline-async carries `Task.Delay`. Tells us whether the timer/scheduler work is needed sooner than expected.
7. **Async-iterator fixture (de-risks E/§3.5).** `await foreach` over an `IAsyncEnumerable`. Likely *exposes* the scheduler gap — valuable precisely because it names what inline-async can't do.
8. **Differential harness spike (de-risks E and measurement).** A script: given a `.cs`/project, build once, run under both runtimes, diff exit code + stdout. This is the reusable oracle every option leans on; building it early pays back immediately. **Status (2026-06-20, Phase 3 + finalizer): ✅ Completed.** `scripts/diff_run.sh` is in-tree, accepts `.cs`, `.csproj`, or prebuilt `.dll` inputs, and is validated; a Rust integration parity smoke test also checks `dotnet` vs `dotnet-rs` for `expression_compile_42`.
9. **xUnit discovery probe (de-risks E test-runner milestone).** Try to load `xunit.core`/`xunit.execution` and enumerate one `[Fact]`. Tells us how close a native test runner is.

**Suggested first sitting:** probes 1, 2, and 8 together — minimal runtime change, and they jointly determine whether EF (the highest-payoff target) is reachable and give us the harness to prove it.

## Appendix: source references

### Codebase (grounding)
- Dispatch / execution loop: `crates/dotnet-vm/src/dispatch/mod.rs`; dispatch codegen: `crates/dotnet-vm/build_support/codegen.rs`.
- Generics: `crates/dotnet-types/src/generics.rs`.
- Exceptions (two-pass SEH): `crates/dotnet-exceptions/src/lib.rs`; `docs/EXCEPTION_HANDLING.md`.
- Intrinsics: `crates/dotnet-intrinsics-*/`; VM-local: `crates/dotnet-vm/src/intrinsics/`; PHF dispatch: `docs/BUILD_TIME_CODE_GENERATION.md`.
- Reflection (incl. `Invoke` re-entrancy): `crates/dotnet-intrinsics-reflection/src/methods.rs:191-263`; `crates/dotnet-vm-data/src/stack.rs:56`; `crates/dotnet-vm/src/stack/context.rs:288`.
- Support library (stubs incl. async builder/Task): `crates/dotnet-assemblies/src/support/` (`AsyncMethodBuilders.cs`, `Task.cs`, `TaskCompletionSource.cs`, `MethodInfo.cs`, `RuntimeType.cs`).
- Loader / runtime discovery: `crates/dotnet-assemblies/src/loader.rs`; `crates/dotnet-assemblies/src/resolution.rs:881-906`.
- CLI args: `crates/dotnet-cli/src/lib.rs:18-29`.
- P/Invoke (libffi/libloading): `crates/dotnet-pinvoke/`.
- GC / threading: `crates/dotnet-vm/src/gc/coordinator.rs`; `crates/dotnet-vm/src/threading/basic.rs`; `docs/GC_AND_MEMORY_SAFETY.md`, `docs/THREADING_AND_SYNCHRONIZATION.md`.
- Test harness / fixtures: `crates/dotnet-cli/build.rs`; `crates/dotnet-cli/tests/integration_tests_impl/harness.rs`; `crates/dotnet-cli/tests/fixtures/`.
- Benchmarks: `crates/dotnet-benchmarks/`.
- Confirmed absent (grep): `Socket`/`HttpClient`/`FileStream`/`System.Net`/`std::net`/`std::fs::File` in intrinsic/VM/pinvoke crates; `IsDynamicCodeSupported`/`RuntimeFeature` anywhere.

### External (Microsoft Learn / .NET)
- Expression trees & execution (interpreter vs emit): https://learn.microsoft.com/dotnet/csharp/advanced-topics/expression-trees/expression-trees-execution
- `LambdaExpression.Compile(bool preferInterpretation)`: https://learn.microsoft.com/dotnet/api/system.linq.expressions.lambdaexpression.compile
- `RuntimeFeature.IsDynamicCodeSupported` / `IsDynamicCodeCompiled`: https://learn.microsoft.com/dotnet/api/system.runtime.compilerservices.runtimefeature.isdynamiccodesupported
- Feature switches / `[FeatureGuard(RequiresDynamicCode)]` (.NET 9): https://learn.microsoft.com/dotnet/core/whats-new/dotnet-9/runtime
- `RequiresDynamicCode` / IL3050 (what truly needs emit): https://learn.microsoft.com/dotnet/core/deploying/native-aot/fixing-warnings
- Reflection.Emit (DynamicMethod / TypeBuilder): https://learn.microsoft.com/dotnet/fundamentals/reflection/emitting-dynamic-methods-and-assemblies
- EF Core query compilation & caching: https://learn.microsoft.com/ef/core/performance/advanced-performance-topics
- EF Core NativeAOT/precompiled queries (the no-dynamic-code direction): https://learn.microsoft.com/ef/core/performance/nativeaot-and-precompiled-queries
- Default assembly probing (`TRUSTED_PLATFORM_ASSEMBLIES`, `deps.json`): https://learn.microsoft.com/dotnet/core/dependency-loading/default-probing
- Framework roll-forward / version selection: https://learn.microsoft.com/dotnet/core/versions/selection
- Host RID-asset selection (portable RID list, no RID graph): https://learn.microsoft.com/dotnet/core/compatibility/deployment/8.0/rid-asset-list
- `dotnet` host options (`--depsfile`, `--runtimeconfig`): https://learn.microsoft.com/dotnet/core/tools/dotnet
- Blazor hosting models (Server/WASM/Hybrid): https://learn.microsoft.com/aspnet/core/blazor/hosting-models
- Blazor build output (no runtime UI compilation): https://learn.microsoft.com/dotnet/architecture/blazor-for-web-forms-developers/project-structure
- Mono interpreter (precedent: dynamic features without JIT): https://learn.microsoft.com/dotnet/maui/macios/interpreter
