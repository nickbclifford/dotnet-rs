# CoreLib `Task` and the thread pool

**Status:** Not started
**Outcome:** Guest code runs CoreLib's own `Task`, `TaskCompletionSource`, awaiters, async
method builders and `ValueTask`, with no `DotnetRs` replacement. Under `multithreading`,
`Task.Run`, `ContinueWith`, `await Task.Yield()`, `Task.WhenAll`, `Task.Delay`, `CancelAfter`
and blocking `.Result`/`Wait()` produce the same results as on the CLR.

## Problem

The VM replaces CoreLib's task types with a single-threaded shim in
`crates/dotnet-assemblies/src/support/`: `Task.cs` (`Task`, `Task<T>`),
`TaskCompletionSource.cs`, `TaskAwaiters.cs` (four awaiters), `AsyncMethodBuilders.cs` (four
builders) and `ValueTask.cs`, each marked `[Stub(InPlaceOf = …)]`. Behavior today:

- `Wait()` and `.Result` on an incomplete task throw "Task has not completed" instead of
  blocking. A `TaskCompletionSource` completed by a worker cannot be awaited synchronously by
  main.
- Completion and continuation registration are unsynchronized (plain fields `_isCompleted`,
  `_exception`, `_continuation`).
- `Task.Run`, `Delay`, `WhenAll`, `WhenAny`, `ContinueWith`, `Task.Yield` and cancellation do not
  exist.
- `.Result` on a faulted task throws the original exception. The CLR throws
  `AggregateException`. `async_await_faulted_task_propagates_42` and
  `async_await_throw_after_await_42` (`crates/dotnet-cli/tests/fixtures/async/`) both call
  `.Result` and encode the shim's behavior.

**Decision (2026-11-09):** run CoreLib's `Task` and delete the shim. `Task` and its neighbors have
no `extern` methods of their own. A spike with the stubs removed and `Debugger.get_IsAttached`
returning `false` (`crates/dotnet-intrinsics-core/src/debugger.rs`, now on `main`) passed 7 of
the 9 async fixtures. In the same spike, a `TaskCompletionSource` completed on another thread while
main blocked on `.Result` worked.

What fails beyond the spike. Call paths are from the .NET 10.0.14 `System.Private.CoreLib` the
VM loads (`/usr/share/dotnet/shared/Microsoft.NETCore.App/10.0.14/`). Checked with `ilspycmd`.

- **Thread pool.** Every `ThreadPool` entry point that touches worker management forwards to
  `PortableThreadPool.ThreadPoolInstance`:
  - `RequestWorkerThread`
  - `NotifyWorkItemComplete`, `NotifyWorkItemProgress` and `GetOrCreateThreadLocalCompletionCountObject`
  - `NotifyThreadBlocked`/`Unblocked` and `ReportThreadStatus`
  - `Get`/`SetMinThreads`, `Get`/`SetMaxThreads` and `GetAvailableThreads`
  - `ThreadCount` and `CompletedWorkItemCount`

  The work queue itself (`ThreadPoolWorkQueue`) is managed and works. The forwarding fails
  through native resources on both sides:
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
  The only unsupported piece is the event:
  - `EventWaitHandle.CreateEventCore`, `Set` and `Reset` call `Interop.Kernel32.CreateEventEx`,
    `SetEvent` and `ResetEvent`.
  - `WaitHandle.WaitOneCore` is the `WaitHandle_WaitOneCore` QCall.
- **Resource strings.** `System.SR::GetResourceString` and `InternalGetResourceString` are
  intercepted by `intrinsic_sr_get_resource_string` (`crates/dotnet-intrinsics-core/src/math.rs`),
  which returns the key. So `new AggregateException().Message` is
  `AggregateException_ctor_DefaultMessage` and not "One or more errors occurred." CoreLib's own
  `SR.InternalGetResourceString` reads `ResourceManager(typeof(Strings))`, a managed
  `.resources` decoder over the embedded manifest resource. The VM already serves manifest
  resources (`crates/dotnet-intrinsics-reflection/src/resources.rs`).

## Scope

In:
- Deleting the five shim files and every VM reference to them.
- A shim for the `ThreadPool` → `PortableThreadPool` forwarding layer (the members listed
  above), and a shim for unnamed `EventWaitHandle`s and their single-handle waits. Each shims the
  managed abstraction above a cluster of same-subsystem native calls, never an individual
  `Kernel32`/`Interop.Sys` call.
- `ExecutionContext` and `AsyncLocal<T>` flow into pool work items and continuations. CoreLib
  implements both in managed code, so this needs fixtures, not new mechanisms.
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
through `ManualResetEventSlim`, which sits on plan 12's `Monitor.Wait`. That makes slice 1 a
deletion plus fixture corrections. The pool and events then serve real CoreLib code.

**Pool workers.** Replace the `ThreadPool` forwarding members, so `PortableThreadPool` is never
constructed:
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
- The shim covers `EventWaitHandle.CreateEventCore(bool, EventResetMode)`, `Set` and `Reset`, and
  `WaitHandle.WaitOneCore(nint, int, bool)` for handles the VM created.
- The `SafeWaitHandle` holds the VM's event ID.
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
   - Re-run the NuGet host-runner rungs, because exception messages change everywhere.

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
