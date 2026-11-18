# CoreLib `Task` and the thread pool

**Status:** Complete
**Outcome:** Guest code runs CoreLib's own `Task`, `TaskCompletionSource`, awaiters, async method
builders and `ValueTask`, with no `DotnetRs` replacement. Under `multithreading`, `Task.Run`,
`ContinueWith`, `await Task.Yield()`, `Task.WhenAll`, `Task.Delay`, `CancelAfter` and blocking
`.Result`/`Wait()` produce the same results as on the CLR.

## What was built

- **CoreLib tasks.** The `DotnetRs` task shim (`Task.cs`, `TaskCompletionSource.cs`,
  `TaskAwaiters.cs`, `AsyncMethodBuilders.cs`, `ValueTask.cs`) was deleted; CoreLib's types run
  unmodified. Blocking waits go through `ManualResetEventSlim` on `Monitor.Wait`
  ([plan 12](12-managed-threading-intrinsics.md)). `Debugger.get_IsAttached` returns `false`.
- **Thread pool.** VM member intrinsics replace CoreLib's `ThreadPool` → `PortableThreadPool`
  forwards (`RequestWorkerThread`, the work-item and thread notifications,
  `GetOrCreateThreadLocalCompletionCountObject`, min/max/available threads, `ThreadCount`,
  `CompletedWorkItemCount`), so `PortableThreadPool` is never constructed. CoreLib's managed
  `ThreadPoolWorkQueue` stays; its `LowLevelLock` uses the VM monitor of the lock object. Workers
  are background executors capped at `max(2, available_parallelism)`; each calls
  `ThreadPoolWorkQueue.Dispatch()` once per request, parks between requests on the condition-wait
  protocol and never retires. A throwing work item is fatal, as on the CLR.
- **Events and timers.** Unnamed `EventWaitHandle`s are VM events (signaled flag and reset mode)
  behind a real non-owning `SafeWaitHandle` with a positive, never-reused ID; single-handle waits
  use the condition-wait protocol. `TimerQueue` and its managed timer thread run unmodified, so
  timer callbacks run on pool workers.
- **`ExecutionContext`, `AsyncLocal<T>` and `[ThreadStatic]`** flow through CoreLib's managed code
  plus VM thread-static storage keyed by logical thread.
- **Resource strings.** The `SR` resource-string intrinsic was deleted; CoreLib's `SR` and
  `ResourceManager` decode the embedded manifest resources
  (`crates/dotnet-intrinsics-reflection/src/resources.rs`).

Details are in [Threading and Synchronization](../THREADING_AND_SYNCHRONIZATION.md#managed-threadpool-work).

## Unsupported (decided, not defects)

- Thread-pool tuning: hill climbing, starvation injection, the gate thread. `Set`/`GetMinThreads`
  and `Set`/`GetMaxThreads` accept and report values but do not resize the pool;
  `GetAvailableThreads` is approximate.
- Named events, `Mutex`, `Semaphore`, `WaitHandle.WaitAny`/`WaitAll` and `SignalAndWait`; they fail
  at their QCalls.
- Green tasks on the effect scheduler, I/O completion ports and `RegisterWaitForSingleObject`.
- Deadlock detection under `multithreading` (every thread parked, nothing pending): with timers on
  a managed thread there is no single place to detect it.
- `SynchronizationContext` implementations; the default `null` context and guest-installed
  contexts follow CoreLib's managed code.
- Without `multithreading` there are no pool workers: `RequestWorkerThread` only records the
  request, `Task.Run(...).Result` reports the infinite-`Monitor.Wait` deadlock error, and
  `Task.Delay` fails when `TimerQueue` tries to start its timer thread. Awaiting already-completed
  tasks works.

## Accepted costs

Measured with `bench-fat`, default `multithreading`, no instrumentation, 30 samples.

- **CoreLib tasks:** `async_await` (128 iterations awaiting a synchronously completed
  `TaskCompletionSource<int>` and a completed `ValueTask<int>`) went from 24.842/24.978 ms with the
  shim to 36.685/36.868 ms (+46–48%). The maintainer accepted it.
- **Managed resources:** `exception_messages` (64 default `AggregateException`s and 64
  `TaskCanceledException`s per run, fresh VM each iteration) went from 3.4581/3.3420 ms with the
  `SR` intrinsic to about 120–124 ms (about 35×). The maintainer accepted it to keep CLR-matching
  messages without a VM-side decoder. The split benchmarks then measured `resource_cold_load`
  96.268/94.917 ms and `resource_warm_lookup` 27.774/28.522 ms. Later numbers and the open
  optimization are in the plans README, "Warm managed resource lookup cost".

```bash
cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- async_await --sample-size 30
cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- exception_messages --sample-size 30
cargo bench --profile bench-fat -p dotnet-benchmarks --bench end_to_end -- resource_ --sample-size 30
```

The NuGet host rungs were rerun after the resource change
([evidence](evidence/corelib-resource-host-rungs.txt)): Expression and EF matched; the Newtonsoft
roundtrip failed before and after in `SZArrayHelper<T>.CopyTo` (fixed by
[plan 18](18-managed-api-clr-gaps.md)).

## Checks

Async, threading and exceptions fixtures (CLR-diffed where possible): `.Result` on a faulted task
throws `AggregateException`; a worker completes a `TaskCompletionSource` while main blocks; pool,
`ContinueWith`, `Task.Yield`, `WhenAll`, `QueueUserWorkItem`, `AsyncLocal` flow, fatal pool
exceptions, event grants, `WaitOne` timeout, `Task.Delay`, `CancelAfter`, `Timer` callbacks and
`WhenAny` timeouts. Deterministic tests cover a `Set` before registration, between registration
and recheck, after parking, and racing a timeout. Run `cargo test -p dotnet-vm --lib` and the whole
`integration_tests` binary in both configurations.
