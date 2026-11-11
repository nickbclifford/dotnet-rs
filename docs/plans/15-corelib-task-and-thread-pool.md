# CoreLib `Task` and the thread pool

**Status:** Complete
**Outcome:** Guest code runs CoreLib's own `Task`, `TaskCompletionSource`, awaiters, async
method builders and `ValueTask`, with no `DotnetRs` replacement. Under `multithreading`,
`Task.Run`, `ContinueWith`, `await Task.Yield()`, `Task.WhenAll`, `Task.Delay`, `CancelAfter`
and blocking `.Result`/`Wait()` produce the same results as on the CLR.

## Problem

The VM previously replaced CoreLib's task types with a limited shim in
`crates/dotnet-assemblies/src/support/`: `Task.cs` (`Task`, `Task<T>`),
`TaskCompletionSource.cs`, `TaskAwaiters.cs` (four awaiters), `AsyncMethodBuilders.cs` (four
builders) and `ValueTask.cs`, each marked `[Stub(InPlaceOf = …)]`. The shim had these limitations:

- `Wait()` and `.Result` on an incomplete task threw "Task has not completed" instead of
  blocking. A `TaskCompletionSource` completed by a worker could not be awaited synchronously by
  main.
- Completion and continuation registration used a lock, but unlocked reads, result publication
  and check-then-act operations did not provide CoreLib's full synchronization semantics.
- `Task.Run`, `Delay`, `WhenAll`, `WhenAny`, `ContinueWith`, `Task.Yield` and cancellation do not
  exist in the shim.
- `.Result` on a faulted task threw the original exception rather than the CLR's
  `AggregateException`; two async fixtures encoded that behavior.

**Decision (2026-11-09):** run CoreLib's `Task` and delete the shim. `Task` and its neighbors have
no `extern` methods of their own. A spike with the stubs removed and `Debugger.get_IsAttached`
returning `false` (`crates/dotnet-intrinsics-core/src/debugger.rs`, now on `main`) passed 7 of
the 9 async fixtures. In the same spike, a `TaskCompletionSource` completed on another thread while
main blocked on `.Result` worked.

Additional dependencies identified after the spike in the loaded .NET 10.0.14
`System.Private.CoreLib` (`/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.14/`), checked with
`ilspycmd`:

- **Thread pool.** CoreLib's `ThreadPool` entry points that touch worker management forward to
  `PortableThreadPool.ThreadPoolInstance`; VM member intrinsics handle these forwards so the
  portable pool is not constructed:
  - `RequestWorkerThread`
  - `NotifyWorkItemComplete`, `NotifyWorkItemProgress` and `GetOrCreateThreadLocalCompletionCountObject`
  - `NotifyThreadBlocked`/`Unblocked` and `ReportThreadStatus`
  - `Get`/`SetMinThreads`, `Get`/`SetMaxThreads` and `GetAvailableThreads`
  - `ThreadCount` and `CompletedWorkItemCount`

  The work queue itself (`ThreadPoolWorkQueue`) is managed, but its queue-assignment lock is a
  `LowLevelLock` whose CoreLib initialization calls `LowLevelMonitor_Create`. The VM intercepts
  initialization and `Acquire`/`TryAcquire`/`Release` through the lock object's monitor, so the
  managed queue does not enter that native path. Without the VM member handlers, portable-pool
  construction and management also require native resources:
  - `PortableThreadPool`'s constructor creates a `LowLevelLock`, whose `LowLevelMonitor` calls
    `Interop.Sys.LowLevelMonitor_Create`.
  - `WorkerThread`'s static `LowLevelLifoSemaphore` needs a native semaphore.
  - `GateThread`'s static `AutoResetEvent`s need `Kernel32.CreateEventEx`.

  `ThreadPoolWorkQueue.Dispatch()` itself reaches the `NotifyWorkItemComplete` and completion-count
  forwards after every work item.
- **Timers.** `TimerQueue` is portable managed code, with no native timer. Its static constructor
  creates `s_timerEvent = new AutoResetEvent(false)`. `SetTimerPortable` signals that event and,
  on first use, starts a background managed `Thread` running `TimerThread`. That thread loops on
  `s_timerEvent.WaitOne(timeout)` and queues expired `TimerQueue`s to the pool with
  `ThreadPool.UnsafeQueueHighPriorityWorkItemInternal`, whose `Execute` runs `FireNextTimers`.
  CoreLib's unnamed event and single-handle wait implementations call native APIs; the VM routes
  these managed members through VM-owned events instead. Named events and multi-handle waits
  remain unsupported.
- **Resource strings.** CoreLib's `SR.InternalGetResourceString` reads
  `ResourceManager(typeof(Strings))`, a managed `.resources` decoder over the embedded manifest
  resource. The VM serves manifest resources (`crates/dotnet-intrinsics-reflection/src/resources.rs`).

## Scope

In:
- Deleting the five shim files and every VM reference to them.
- A shim for the `ThreadPool` → `PortableThreadPool` forwarding layer (the members listed
  above), a VM-monitor-backed CoreLib `LowLevelLock` for the work queue's queue-assignment
  critical section, and a shim for unnamed `EventWaitHandle`s and their single-handle waits. Each
  shims the managed abstraction above a cluster of same-subsystem native calls, never an
  individual `Kernel32`/`Interop.Sys` call.
- `ExecutionContext` and `AsyncLocal<T>` flow into pool work items and continuations. CoreLib
  implements the flow in managed code. The pool slice also needs VM thread-static storage:
  CoreLib's queue locals and current-task identity rely on `[ThreadStatic]`, including byrefs.
- CoreLib resource strings through CoreLib's own `SR` and `ResourceManager`.

Out:
- **Thread-pool tuning:** hill climbing, starvation injection, the gate thread, and
  `GetAvailableThreads` accuracy. Valid programs can't observe the worker count except through
  timing. `Set`/`GetMinThreads` and `Set`/`GetMaxThreads` accept and report values but don't
  resize the pool.
- **Named events, `Mutex`, `Semaphore`, `WaitHandle.WaitAny`/`WaitAll` and `SignalAndWait`:**
  timers and the pool don't need them. They keep failing at their QCalls.
- **Green tasks on the effect scheduler.** They would be deterministic, but they need a scheduler
  mechanism plan 08 does not have.
- **I/O completion ports and `RegisterWaitForSingleObject`:** these have no consumer in the
  fixtures or NuGet rungs.
- **Deadlock detection under `multithreading`** (every thread parked, nothing pending). With
  timers running on a managed thread, there is no single place to detect it. It stays
  unplanned.
- **`SynchronizationContext` implementations:** there is no UI framework to install one. The
  default `null` context, and a guest-installed context, follow CoreLib's managed code.
- **A thread pool without `multithreading`:** see Design.

## Design

**Delete the shim first.** Real `Task` blocking already works: `Task.InternalWaitCore` waits
through `ManualResetEventSlim`, which sits on plan 12's `Monitor.Wait`. Before waiting,
`Task.SpinThenBlockingWait` calls `ThreadPool.NotifyThreadBlocked`, which otherwise constructs
`PortableThreadPool`. The maintainer approved implementing only
`NotifyThreadBlocked` (returning false) and `NotifyThreadUnblocked` (no-op) alongside the task
deletion. All other pool forwards and workers remain in slice 2. The pool and events then serve
real CoreLib code.

**Pool workers.** Replace the `ThreadPool` forwarding members, so `PortableThreadPool` is never
constructed:
- CoreLib's managed `ThreadPoolWorkQueue` also constructs a `LowLevelLock`. Its initialization
  and `Acquire`/`TryAcquire`/`Release` operations use the VM monitor associated with the lock
  object, rather than entering the native `LowLevelMonitor` API.
- `RequestWorkerThread` adds one outstanding request. An idle pool worker takes it. If no worker
  is idle and the pool is below its cap, the VM starts a new one. The cap is the host's
  available parallelism, with a minimum of 2.
- A worker satisfying a request calls CoreLib's `ThreadPoolWorkQueue.Dispatch()` once. The VM
  resolves the method by name, as it resolves `DotnetRs.ThreadStartRunner` in
  `crates/dotnet-vm/src/stack/context_ops.rs`. `Dispatch` requests another thread itself
  (`EnsureThreadRequested`) when more work remains.
- Between requests a worker parks on the slice-5 condition-wait protocol. Idle workers never
  exit, so the pool adds no retired-worker cost beyond its cap.
- `NotifyWorkItemComplete` returns `true` (keep dispatching). The progress, blocked/unblocked and
  status notifications do nothing. `GetOrCreateThreadLocalCompletionCountObject` returns a shared
  placeholder object. `ThreadCount` reports the live workers, and `CompletedWorkItemCount` a
  counter the worker increments.
- Workers are `IsBackground` (plan 12), so exit does not wait for them.
- A pool work item that throws reaches the worker's unhandled-exception path and is fatal
  (plan 11), as on the CLR. `Task` bodies catch their own exceptions inside `Task.ExecuteEntry`.

**Events.** Back unnamed `EventWaitHandle`s with a VM event: a signaled flag plus a reset mode.
- The shim covers `EventWaitHandle.CreateEventCore(bool, EventResetMode)`, instance `Set` and
  `Reset`, static `Set(SafeWaitHandle)`, and `WaitHandle.WaitOneCore(nint, int, bool)` for handles
  the VM created.
- A real non-owning `SafeWaitHandle` holds the VM's positive, non-reused event ID. The registry
  retains event records until VM teardown; disposing a wrapper never closes a native handle.
- Waits use the condition-wait protocol with an `Instant` deadline, like plan 12's
  `Monitor.Wait`. An auto-reset event grants exactly one waiter per `Set`.
- `TimerQueue` and its timer thread then run unmodified. The timer thread's timed `WaitOne` is
  the deadline source, and timer callbacks run on pool workers as on the CLR.
- This also makes guest `AutoResetEvent` and `ManualResetEvent` usable, a side benefit. It is not
  a goal.

**Without `multithreading`:** there are no managed workers.
- `RequestWorkerThread` records the request and does nothing else.
- A blocking wait that depends on queued work hits plan 12's infinite-`Monitor.Wait` deadlock
  error.
- `Task.Delay` fails when `TimerQueue` tries to start its timer thread, with the existing
  "requires the multithreading feature" error.
- `async`/`await` over already-completed or synchronously-completed tasks keeps working, because
  those continuations run inline.

Draining the queue inline at a park point would be green scheduling, which is out of scope.

**Resource strings:** delete `intrinsic_sr_get_resource_string`, so CoreLib's `SR` and
`ResourceManager` decode its own embedded resources. `SR` already guards against recursive
lookups. Measure: lookups now run managed decoding on the first use of each key, and exception
construction on hot paths could slow down.

## Slices

1. **Run CoreLib's task types.**
   - Before deleting anything, record a baseline for the shim on `main` that later slices can
     compare against. Add an async case to the `end_to_end` benchmark: an `await` loop over a
     `TaskCompletionSource` and a completed `ValueTask`. Record the shim's numbers in this plan.
     Once the shim is gone, this baseline cannot be recovered.
     - **Recorded shim baseline (2026-11-11):** `async_await` in
       `crates/dotnet-benchmarks/fixtures/async_await/AsyncAwaitBenchmark_0.cs` executes 128
       iterations per fixture run. Every iteration awaits an incomplete `TaskCompletionSource<int>`
       operation, completes its source synchronously on the caller thread (and verifies that the
       task was incomplete before completion), then awaits an already-completed `ValueTask<int>`.
       It returns exit code 0 only for the stable accumulated result 16,512. It uses no worker or
       timer. The benchmark command, with the benchmark crate's default `multithreading` feature
       enabled and `bench-instrumentation` disabled, was run twice:
       `cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- async_await --sample-size 30 --save-baseline task-shim`.
       - First run: estimate 24.842 ms, Criterion interval [24.734 ms, 24.958 ms].
       - Repeat: estimate 24.978 ms, interval [24.871 ms, 25.098 ms]. Criterion's repeat-vs-saved
         baseline comparison was [-0.0813%, +0.5472%, +1.1955%], p = 0.10; it reported no detected
         performance change (2/30 high-mild outliers). These overlapping intervals and the
         repeat comparison are the measured shim-run variability, not a post-deletion comparison.
     - **Measurement environment:** x86_64 Arch Linux (`7.2.14-arch1-1`), AMD Ryzen 9 9900X
       (12-core/24 logical CPUs); Rust 1.95.0; .NET SDK 10.0.114 and the .NET 10.0.14
       `Microsoft.NETCore.App` runtime/CoreLib. Profile `bench-fat` (LTO fat, one codegen unit,
       inherited release overflow checks), default `dotnet-benchmarks` `multithreading` feature;
       no `bench-instrumentation`. CLR verification used the same SDK/runtime; the freshly built
       no-feature VM binary also passed `scripts/diff_run.sh --host` with matching exit code and
       stdout. At measurement time HEAD was `9a382f342585c173620fc21a40d77c336e71b147` and
       `main` was `7f312de53fe3f8a55020b0cafaecd91ed4e3052f`; HEAD's kickoff-review commit changed
       documentation only, and benchmark/task-support sources were unchanged from `main`. The five
       task support substitutions remained present throughout both measurements.
     - **CoreLib task measurement (2026-11-11):** On the same host, runtime, profile, features
       and unchanged workload, after removing the five substitutions at HEAD `0160d9e8`, ran twice:
       `cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- async_await --sample-size 30 --baseline task-shim`.
       First: 36.685 ms [36.625, 36.750], change [+46.143%, +46.868%, +47.551%], p < 0.05.
       Repeat: 36.868 ms [36.759, 37.023], change [+46.734%, +47.601%, +48.471%], p < 0.05.
       Both are well beyond the measured shim variability. The maintainer approved keeping CoreLib
       tasks despite the measured cost; the task shim is not retained.
   - Delete `Task.cs`, `TaskCompletionSource.cs`, `TaskAwaiters.cs`, `AsyncMethodBuilders.cs` and
     `ValueTask.cs`.
   - Rewrite the two shim-encoding fixtures to the CLR's behavior. `.Result` on a faulted task
     catches `AggregateException` and checks `InnerException`; `await` still observes the
     original exception.
   - Add a fixture (`multithreading`) where a worker completes a `TaskCompletionSource` while main
     blocks on `.Result`, and one where main blocks on `Wait(timeout)` that times out.
   - Run the benchmark against the baseline. A regression above noise is a maintainer question,
     not a reason to keep the shim.
2. **Thread pool.**
   - Add the forwarding-layer shim and the background pool workers.
   - Fixtures (`multithreading`), each compared with the CLR through `scripts/diff_run.sh`:
     - `Task.Run(...).Result`
     - `ContinueWith`
     - `await Task.Yield()` in an async `Main`
     - `Task.WhenAll` over several `Task.Run`s
     - `ThreadPool.QueueUserWorkItem` with a `ManualResetEventSlim`
     - `AsyncLocal<T>` flowing into `Task.Run` and across an `await`
     - an exception thrown from a `QueueUserWorkItem` callback terminating the process
   - Without `multithreading`: a fixture that `await`s completed tasks still passes, and
     `Task.Run(...).Result` reports the deadlock error.
3. **Events and timers.**
   - Add the unnamed-event shim.
   - Fixtures (`multithreading`):
     - `AutoResetEvent` granting one waiter per `Set`, and `ManualResetEvent` releasing all
       waiters
     - `WaitOne(timeout)` returning `false`
     - `await Task.Delay(50)` with a lower bound on elapsed time
     - `CancellationTokenSource.CancelAfter` cancelling a `Task.Delay`
     - a `System.Threading.Timer` callback firing on a pool thread
     - `Task.WhenAny(work, Task.Delay(...))` timing out
   - Deterministic tests: a `Set` before registration, between registration and recheck, and
     after parking; and a timeout racing a `Set`. These mirror plan 12's `Monitor.Wait` tests.
4. **CoreLib resource strings.** Delete `intrinsic_sr_get_resource_string`.
   - Fixture: the default `AggregateException.Message` text and `TaskCanceledException.Message`
     match the CLR.
   - Measure an exception-heavy fixture before and after.
     - **Resource measurement (2026-11-11):** `exception_messages` throws 64 default
       `AggregateException`s and 64 default `TaskCanceledException`s and checks each message is
       nonempty under invariant UI culture. Each Criterion iteration constructs a fresh VM and
       static state, loads the entry assembly, performs the cold first `ResourceManager` lookup
       and 127 subsequent lookups; this is a cold-inclusive end-to-end measurement, not an
       isolated hot-lookup measurement. The harness reuses its prepared loader and fixture DLL.
       Compilation is outside the timed section. On x86_64 Arch Linux 7.2.15, Ryzen 9 9900X,
       Rust 1.95.0, .NET SDK 10.0.114 and CoreLib 10.0.14, `bench-fat` with default
       `multithreading` and no instrumentation, the pre-removal command was
       `cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- exception_messages --sample-size 30 --save-baseline resource-shim`:
       3.4581 ms [3.3914, 3.5338]. The repeat used `--baseline resource-shim`: 3.3420 ms
       [3.3142, 3.3664], Criterion change [-3.3928%, -1.9726%, -0.6831%], within its noise
       threshold. The first run overlapped other build/host work; the repeat did not.
       After removal, the same `--baseline resource-shim` command gave 120.36 ms
       [119.03, 121.63], change [+3402.0%, +3464.6%, +3521.6%], and 122.84 ms
       [121.39, 124.18], change [+3478.9%, +3538.0%, +3600.4%]; both p < 0.05. The
       cold-inclusive regression is well beyond observed noise and requires a maintainer decision.
       Repeating after the fuzz-sandbox compatibility fix gave 121.64 ms [120.40, 122.81],
       change [+3442.6%, +3502.7%, +3558.1%], and 120.74 ms [119.39, 122.03],
       change [+3412.6%, +3475.9%, +3535.0%], both p < 0.05. After adding the split
       harness, the same baseline command gave 124.29 ms [123.06, 125.52], change
       [+3522.6%, +3581.2%, +3639.9%], p < 0.05.
       HEAD was `0a6f2d6b004cbffea3cede914d0ce385f74d3f7e`; main was
       `7f312de53fe3f8a55020b0cafaecd91ed4e3052f`. The fixture and host were identical.
     - **Maintainer decision (2026-11-11): accepted.** Keep managed CoreLib resources (no key
       fallback, no VM decoder); the ~35x cold-inclusive regression is accepted for this slice.
       Condition: split the benchmark into cold-load and warm-lookup variants so the cost can be
       attributed (one-time startup vs per-lookup), and track the optimization in the plans
       README "Not yet planned" entry "Managed resource lookup cost". Keep the
       `exception_messages` fixture as the baseline.
     - **Cold/warm attribution (2026-11-11):** Same host, profile and features as above.
       `cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- resource_ --sample-size 30 --save-baseline resource-split`
       and repeat with `--baseline resource-split`. `resource_cold_load` creates a fresh VM and
       performs the first aggregate and cancellation message lookups; the loader and fixture DLL
       are already prepared. It measured 96.268 ms [95.459, 96.939] and 94.917 ms
       [93.867, 95.833], repeat change [-2.6490%, -1.4030%, -0.0327%], within Criterion's
       noise threshold. `resource_warm_lookup` primes both messages once outside timing and
       reenters a void fixture in the same VM for 128 exceptions and lookups per iteration. It
       measured 27.774 ms [27.752, 27.796] and 28.522 ms [28.479, 28.569], repeat change
       [+2.5157%, +2.6928%, +2.8946%]. These runs show substantial cold cost as well as a
       nontrivial repeated cost; warm timings include exception creation, throwing, entrypoint
       and message validation, not just lookup. The cold and warm fixtures and VM lifetimes
       differ, so subtraction is only indicative, not a precise decomposition.
   - Re-run the NuGet host-runner rungs, because exception messages change everywhere. The
     [host-run evidence](evidence/corelib-resource-host-rungs.txt) records before/after stdout,
     stderr and exit results in both feature configurations. Expression and EF matched; the
     pinned Newtonsoft roundtrip failed before and after in `SZArrayHelper<T>.CopyTo`.

## Done when

- The five shim files are gone and nothing in the VM refers to the stubbed task types.
- `PortableThreadPool` is never constructed, and no fixture reaches `Kernel32.CreateEventEx`,
  `LowLevelMonitor_Create` or the `WaitHandle_WaitOneCore` QCall.
- Every fixture above passes in the configurations it applies to, and matches the CLR where the
  differential harness can compare it.
- In both configurations:
  - the `async`, `threading` and `exceptions` fixture selectors (for example
    `cargo test -p dotnet-cli --test integration_tests async`)
  - `cargo test -p dotnet-vm --lib`
  - the whole `integration_tests` binary
  - clippy with `-D warnings` for the touched crates
- The async benchmark's numbers before and after are recorded here.
- `docs/THREADING_AND_SYNCHRONIZATION.md` describes the pool workers and the event shim. Its
  "ThreadPool scheduling remain unsupported" sentence is removed.
- The plans README "Retired-worker resource cost" entry notes that pool workers are capped and
  never retire.

## Ask the maintainer only if

- A slice needs a scheduler mechanism plan 08 does not have, beyond condition-wait wakes and
  deadlines (for example, green tasks, or running pool work inline on a parked thread).
- The non-`multithreading` build needs pool or timer behavior other than what Design states.
- The real `Task` path, or managed resource lookup, regresses a benchmark beyond noise.
- `ExecutionContext` or `AsyncLocal` flow needs VM support beyond CoreLib's managed code.
- CoreLib's `ResourceManager` path fails in a way that needs a VM-side `.resources` decoder.
- A consumer needs named events, `WaitAny`/`WaitAll`, `Mutex` or `Semaphore`.
