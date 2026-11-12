# Plans

The project goal is a maintainable ECMA-335 runtime whose supported behavior and safety
boundaries are understandable. Prefer one working runtime path over more infrastructure,
audits or documents.

## Queue

In priority order. Each plan's own `Status:` line is the only record of its status.

| ID | Plan | Note |
|---|---|---|
| 16 | [Retire legacy continuations and move suspending intrinsics to effects](16-retire-legacy-continuations.md) | In progress; slices 1–6 complete; next is slice 7 (static-init triggers) |
| 15 | [CoreLib `Task` and the thread pool](15-corelib-task-and-thread-pool.md) | CoreLib tasks/resources, capped background pool workers, unnamed events and managed timers are active |
| 14 | [Canonical reflection objects across threads](14-canonical-reflection-objects.md) | Complete; shared canonical type, member and assembly objects |
| 13 | [Async intrinsic API: the body is the implementation](13-async-intrinsic-api.md) | Complete; body validation, inline dispatch and optional fallbacks verified; measured Substring fast path retained |
| 12 | [Managed threading intrinsics](12-managed-threading-intrinsics.md) | Complete; managed threads and monitor condition waits are supported |
| 11 | [Exception control flow: handlers inside handlers, unhandled worker exceptions](11-exception-control-flow.md) | Complete; nested handlers, fatal worker exceptions and filter exception boundary verified |
| 09 | [Retain arenas after managed thread exit](09-retired-arena-lifetime.md) | Establishes the retained-arena lifecycle contract |
| 05 | [Safe APIs whose soundness rests on unchecked caller premises](05-safe-api-premises.md) | Validates typed field and memory-operation premises |
| 06 | [Cross-arena collection fixture through production decoding](06-cross-arena-collection-fixture.md) | Fixtures use production storage decoding |
| 07 | [Nested filters keep earlier suspended frames](07-nested-filter-suspension.md) | Nested filter segments preserve suspended frames |
| 08 | [Async/await VM effects](08-async-effects.md) | Current paths and slice evidence are in [08-async-effects/](08-async-effects/) |
| 10 | [Async effects hardening](10-async-effects-hardening.md) | Complete; post-merge contract and documentation gaps in 08 closed |
| 04 | [Native continuations: Substring proof](04-native-continuations.md) | Measured large-call cost accepted; scheduler contract and behavior controls retained for 08 slice 1 |
| 03 | [Repair enum and named scalar frame homes](03-frame-home-enum-representation.md) | Done; scalar bounds, opcode/growth coverage and historical reproductions verified |
| 02 | [Stable homes for addressable locals and arguments](02-stable-frame-homes.md) | Done; measured primitive cost accepted |
| 01 | [Executor teardown ownership](01-executor-teardown-ownership.md) | Done; kept as a pointer to the contract |

Plan IDs normally follow dependency order. Never reuse an ID; a repair or prerequisite found
while executing a plan gets the next free ID, even when it must precede that plan.

## Not yet planned

Known problems without a plan. Write a plan for one only when it is chosen as the next work.
Evidence and IDs (p01…, R01…) are defined in the
[pointer-provenance inventory](../pointer-provenance/INVENTORY.md).

- **Managed resource lookup cost:** after plan 15 removed the `SR` resource-string intrinsic, the
  `exception_messages` benchmark (cold first `ResourceManager` load plus 127 lookups, fresh VM
  per iteration) went from about 3.4 ms to about 121 ms (~35x). Accepted by the maintainer for
  correctness (CLR-matching messages, no VM-side decoder). Fresh-VM first lookups account for
  substantial cost and repeated exception/message lookups also cost time; profile (samply) and
  optimize. The warm case includes exception handling rather than isolating dictionary lookup.
  Numbers and commands are in
  [plan 15](15-corelib-task-and-thread-pool.md).
- **Newtonsoft array copying:** the pinned host roundtrip in
  `crates/dotnet-cli/tests/host_projects/newtonsoft_roundtrip` fails in
  `DotnetRs.SZArrayHelper<T>.CopyTo` with `InvalidCastException` while initializing
  `JsonWriter`; the CLR serializes and deserializes successfully.
- **Reflection compatibility and validation:** reflection calls remain transparent (guest
  exceptions are not wrapped in `TargetInvocationException`) and declared byref returns are
  rejected, both deliberate CLR deviations in the
  [plan 08 reflection contract](08-async-effects.md#decisions-already-made). Argument binding
  follows the CLR for counts, targets, widening, `null` and `Nullable<T>`, with these gaps:
  array-typed arguments and receivers are accepted without a type check, a byref argument must
  match its parameter exactly (the CLR widens and writes back), `Nullable<T>` accepts only a
  boxed `T` (no widening), and `Activator.CreateInstance(Type, object[])` and
  `GetConstructor(Type[])` return the first best match rather than throwing
  `AmbiguousMatchException`. `Activator.CreateInstance(typeof(string), args)` throws
  `ArgumentException("Arg_MustBeType")`; the CLR runs the string constructor, which the VM
  implements only through the intrinsic-constructor `newobj` protocol.
- **Reflection byref write-back after throws:** byref arguments are written back when the invoked guest method throws, unlike the CLR.
- **Legacy static-initialization triggers:** `newobj`, eligible calls through
  `prepare_method_initialization` and `RuntimeHelpers.RunClassConstructor` retain the legacy
  `unwind_frame` wrapper, after outer filters have already run on the original exception. Their
  timing and wrapping differ from the field-instruction effect path described in the
  [static-initialization slice](08-async-effects/slice-4-static-initialization.md).
- **Fallback continuations for excluded shapes:** `MulticastState` remains for ref-struct
  values and byref targets containing managed pointers, byref returns and `TypedReference`.
  Non-null heap, frame-home, static, unmanaged and MT cross-arena byref arguments with
  pointer-free targets use native effects; transient byrefs are invalid. The managed-body-only
  cctor, span equality slow path and Expression.Compile routes remain non-inline. Non-inline
  intrinsics use a registered fallback or return `NotImplemented` when absent. Substring retains
  its measured valid-small fast path ([plan 13](13-async-intrinsic-api.md#substring-fast-path-measurement)).
- **Cross-thread GC handles:** `intrinsic_gchandle_internal_get` and related operations index the
  calling arena's local handle table, so a `WeakReference` created on one thread cannot reliably
  read its target from another, even while both arenas remain registered.
- **Finalization and GC compatibility:** finalizers run on the interrupted mutator with its
  thread identity, so they can re-enter monitors it holds or observe an in-progress `.cctor`
  through recursion. A finalizer run by a retired worker can receive a new `Thread` object with
  the worker's ID after the original thread object has been released. The resurrection and
  weak-handle fixtures also disagree with the CLR and may be timing-dependent; resolving these
  behaviors requires separate GC/finalizer work. One such dependence is fixed: a cross-arena
  short `WeakReference` no longer survives the first `GC.Collect` depending on which arena
  finalized first (the `ClearWeak` phase).
- **Exit waits for never-returning finalizers:** `Executor::run` waits until no worker is running
  or finalizing, so a finalizer that never returns blocks every caller. The CLR does not wait for
  finalizers at exit. `Executor::run_entrypoint` returns without the wait (plan 11).
- **Negative and wide managed-pointer offsets:** `ManagedPtr` stores a compact unsigned offset,
  so byref arithmetic that goes below the base or past the compact range (for example
  `Unsafe.Subtract`, or `Unsafe.Add` with a negative count) fails with `NotImplemented`
  (`managed_offset_error` in `crates/dotnet-value/src/stack_value.rs`).
- **Static field addresses:** `ldsflda; conv.u` writes and static payload alignment
  (`p12_ldsflda_convu_stind_static`).
- **Instance field alignment:** `FieldStorage` uses `Vec<u8>`, which does not promise the
  alignment required by `get_field_mut_local`; exception stack-trace writes can trip its
  alignment assertion under Miri.
- **Native call boundary:** payload addresses, ABI returns, by-ref/out copy-back and pinned
  lifetime for P/Invoke (R06/R07). The current cross-arena raw-escape and by-value
  managed-pointer aggregate rejections stay until this is done.
- **Shared raw-memory access:** one protocol for competing plain, volatile, atomic, block and
  raw-alias accesses to shared storage (heap, statics) under `multithreading` (R09/R12).
  `FieldStorage`'s aligned byte-to-integer atomic path also discards pointer provenance when
  storing object references; a subsequent static-storage GC trace fails under Miri.
- **Safe byte writes into reference slots:** `Vector::get_mut`, `FieldStorage::with_data_mut` and
  `ObjectRef::with_data_mut` hand out `&mut [u8]` over storage the collector traces, so safe code
  can write forged pointer bytes; `ObjectPtr::with_data_mut` also skips the owner barrier
  (`ThreadSafeLock::borrow_mut_unbarriered`). Fold into R09/R12 and R10/R11.
- **Resumable bulk operations and barriers:** `cpblk`/`initblk`/`Memmove`/`Array.Copy`/`Fill`
  that keep progress across safepoints and apply GC write barriers to reference slots (R10/R11).
- **Retired-worker resource cost:** each completed explicit managed worker retains one OS thread
  and one arena until VM shutdown; process-wide arena IDs are capped at 65,535. Pool workers never
  retire between requests and are capped at `max(2, available_parallelism)`. Earlier safe retirement
  remains unplanned.
- **Thread-static retention:** thread-static storage and its object references remain rooted until
  VM shutdown, including after an explicit worker exits, so outstanding byrefs remain valid.
  Reclaiming these values when no byref can reach them remains unplanned.
- **`Span<byte>.Clear()` is very slow on large spans:** clearing 3 MiB takes about 42 s and never
  reaches the `SpanHelpers.ClearWithoutReferences` intrinsic, so it runs as interpreted IL
  (found in [plan 16](16-retire-legacy-continuations.md) slice 6; present before it). Find the
  path `Clear` takes in this CoreLib and route it to the intrinsic.
- **String benchmark is code-placement sensitive in MT:** `string` moved +7% in plan 16 slice 6
  (and +33%, then −8%, in slice 3) with unchanged instruction counts, because its hot
  `IsNullOrWhiteSpace` loop shifts across cache-line boundaries as the binary changes. Compare
  `perf stat` instructions before attributing a `string` change to a commit; consider aligning
  hot intrinsic loops if this keeps masking real changes.
- **Timer pool-thread fixture under load:** `threading_timer_callback_pool_thread_42` failed
  twice during full parallel `dotnet-cli` integration runs on 2026-11-11 (once single-threaded,
  once during plan 16 slice 4 validation) and passed every isolated and repeated run. The
  failure message was not captured; reproduce under load and record it before diagnosing.
- **Reflection race witness:** `invoke_argument_race_42` uses bounded concurrent churn, so the
  CLR can finish without observing the intended `ArgumentException` and its differential sanity
  check can fail despite correct execution.
- **Other missing or incorrect runtime behavior:**
  - Buffered stdout lost on an internal-error exit: not reproduced. The VM has no stdout buffer
    of its own (`Console` writes reach `write(2)` through the native shim, and the CLI flushes
    stdio on every exit path), so a repro (a fixture that writes, then fails with `Internal VM
    error`) is needed first.
  - `Delegate.GetInvocationList` is missing on `System.Delegate` (only `MulticastDelegate`
    has it), and `delegate is MulticastDelegate` is false for a runtime delegate type.
  - A `Synchronized` instance method on a value type runs without taking a monitor.
  - `Type.ToString()` returns `"Type: Name"` where the CLR returns the type name, so
    `$"{obj.GetType()}"` and a boxed struct's default `ToString()` (through
    `ValueType.ToString`) differ from the CLR.
  - `Type.IsByRefLike` always returns `false`; reflection invocation now detects byref-like
    types (`invoke_args::is_byref_like`), but the property does not use it.
- **`GC.WaitForPendingFinalizers` global quiescence:** it waits until no arena has pending or
  in-progress finalizers, rather than completing one pass, and can livelock while a thread keeps
  producing finalizable garbage. The behavior has not been reproduced.
- **Loom model coverage:** the hand-written `loom_stw.rs` model does not compile the production
  `WorkerRegistry` and does not cover every stop-the-world, retirement or shutdown interleaving.
- **CI feedback:** remaining steps of the [test hierarchy](../CI.md#test-hierarchy-and-migration).

## Writing a plan

A plan is a short, current document an agent can act on. Keep detailed history in git.
Retain validation conclusions, reproducible check commands, accepted costs and unresolved
limits in the relevant plan or subsystem doc. The pointer inventory records current
safety coverage; dated transcripts do not define current requirements.

- **Header:** `Status:` (Not started / In progress / Parked / Complete) and a one-sentence
  **Outcome** that a test can check.
- **Problem:** what goes wrong today, with code paths and test names.
- **Scope:** what is in, and what is out with the reason. Use the spec (ECMA-335 section
  numbers) to rule out cases that valid programs cannot produce.
- **Design:** the chosen approach, briefly. Reuse existing mechanisms where one fits.
- **Slices:** ordered increments. Each slice ends with the VM using the new path and its tests
  enabled. A slice never builds support that only a later slice uses.
- **Done when:** the checkable exit conditions.
- **Ask the maintainer only if:** a short list of specific triggers.

If a slice turns out to need much more than its description (new modules, or support for
cases outside the stated scope), stop and revise the plan rather than splitting the work into
more preparatory steps.
