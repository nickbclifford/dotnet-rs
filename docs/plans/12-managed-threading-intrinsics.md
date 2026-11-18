# Managed threading intrinsics

**Status:** Complete
**Outcome:** A guest program can use the common `System.Threading.Thread` and `Monitor` surface
(`Yield`, `SpinWait`, `Sleep`, `CurrentThread`, `ManagedThreadId`, `IsBackground`,
`Start(object)`, closure targets, `Join(int)`, `Monitor.Wait`/`Pulse`/`PulseAll`) with CLR-matching
results, and no thread's `Sleep` or wait delays another thread's collection.

## Supported surface and design

These shims sit at the managed API, above CoreLib's QCalls (`crates/dotnet-intrinsics-threading/src/thread.rs`,
`monitor.rs`); none calls `std::thread::sleep` inside a mutation.

- **`Sleep`, `Yield`, `SpinWait`** use the plan 08 condition-wait protocol: `Sleep` parks until
  its deadline or abort outside the mutation (`Sleep(0)` yields once, `Timeout.Infinite` parks
  until abort, other negatives throw `ArgumentOutOfRangeException`); `SpinWait(n)` is a bounded
  yield. `Yield` returns `true` after yielding, because the host cannot report whether another
  thread ran. Non-inline dispatch returns `NotImplemented` without consuming operands.
- **Thread identity.** Each managed thread owns one `Thread` object (created lazily for the entry
  thread, recorded at `Start` for workers); `CurrentThread` returns it, and `ManagedThreadId` is
  unique, stable and distinct from the OS id. Arena-local records root the thread, delegate and
  start argument until completion. `Start(object)` and instance (closure) targets go through the
  ordinary delegate path. `Join(int)` times out or succeeds.
- **`IsBackground`** is stored in the worker registry; background workers are excluded from the
  foreground exit wait and stopped through the abort signal.
- **`Monitor.Wait`/`Pulse`/`PulseAll`.** `SyncBlock` keeps a waiter queue: `Wait` releases all
  recursion, parks, reacquires through `monitor_enter` and restores the count; non-owner calls
  throw `SynchronizationLockException`. In ST, `Wait(timeout)` returns `false` after the timeout
  and an infinite `Wait` reports a deadlock instead of hanging.
- **Fatal entry-thread exception.** An unhandled exception on the entry thread is recorded through
  `SharedGlobalState::report_fatal_exception` before `run` waits for workers, so workers stop at
  their next safepoint (as worker exceptions do since [plan 11](11-exception-control-flow.md)).

Without `multithreading` there are no workers; `Yield`, `SpinWait` and `Sleep` still apply.

Out of scope: `Thread.Abort`/`Interrupt`, priorities and apartment states (see
[Threading and Synchronization](../THREADING_AND_SYNCHRONIZATION.md#asynchronous-exceptions-threadabort-and-threadinterrupt)).
Thread-pool work, timers, tasks and `[ThreadStatic]` storage came later in
[plan 15](15-corelib-task-and-thread-pool.md).

## Checks

Threading fixtures with CLR comparisons where the harness can compare them; deterministic executor
tests for deadline expiry, abort, GC stops while parked, and pulse-before/between/after
registration and timeout races. As [plan 11](11-exception-control-flow.md) explains, run
`cargo test -p dotnet-vm --lib` and the whole `integration_tests` binary in both configurations,
not only the `threading` and `exceptions` selectors.
