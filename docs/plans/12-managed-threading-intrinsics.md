# Managed threading intrinsics

**Status:** Not started
**Outcome:** A guest program can use the common `System.Threading.Thread` and `Monitor` surface
(`Yield`, `SpinWait`, `Sleep`, `CurrentThread`, `ManagedThreadId`, `IsBackground`,
`Start(object)`, closure targets, `Join(int)`, `Monitor.Wait`/`Pulse`/`PulseAll`) with CLR-matching
results, and no thread's `Sleep` or wait delays another thread's collection.

## Problem

Reproduced on `main` (2026-11-09, `multithreading`) with throwaway fixtures that are not in the
tree. Each returns 42 on the CLR.

| Shape | Result today |
|---|---|
| `Thread.Yield()` | `DllNotFoundException: Unable to load DLL 'QCall'` |
| `Thread.SpinWait(100)` | same |
| `Monitor.Wait(o, 10)` and `Monitor.Wait(o)` | same (`Monitor.Wait(ObjectHandleOnStack, int)`) |
| `Monitor.Pulse` / `PulseAll` | not reached; no intrinsic |
| `Thread.CurrentThread == Thread.CurrentThread` | `false`: each call allocates a new `Thread` |
| a worker's `Thread.CurrentThread == startedThread` | `false` |
| `Thread.CurrentThread.ManagedThreadId` | `Missing intrinsic implementation … get_ManagedThreadId()` |
| `t.IsBackground = true` | `ThreadStateException: Argument_InvalidHandle` from `GetNativeHandle` |
| `new Thread(new ParameterizedThreadStart(f)).Start(42)` | same `ThreadStateException` |
| `new Thread(() => { x = 42; })` | `PlatformNotSupportedException: ThreadStart delegates with a target instance are not supported` |
| `Thread.Sleep(3000)` on a worker, `GC.Collect()` on main | collection takes longer than 1 s: `intrinsic_thread_sleep` calls `std::thread::sleep` inside the mutation, so the sleeper cannot join a stop-the-world |

A `ManualResetEventSlim` set by a worker and waited on by main already works.

**Main-thread unhandled exceptions leave the process running.** Plan 11 made a worker's unhandled
exception fatal. The entry thread's is not: with `lock (O) { throw … }` in `Main` and a worker
that does `lock (O)`, the VM reports the exception and then hangs. Search finds no handler, so
`handle_exception` clears the frames without running `finally` blocks and the lock stays held;
`Executor::run` then waits forever for the foreground worker. Without the lock the same program
exits, because the worker finishes. The CLR terminates the process at the unhandled exception.

The ST configuration has no workers (`Managed Thread(ThreadStart) requires the multithreading
feature`), so every shape above that needs a second thread is `multithreading`-only. `Yield`,
`SpinWait` and `Sleep` also apply to ST.

## Scope

In:
- The shapes in the table, plus `Thread.Sleep(TimeSpan)` and `Sleep(Timeout.Infinite)`
  (currently a silent no-op).
- The `IsBackground` exit rule: the process does not wait for background threads, and they
  stop when the process exits.
- Shim at the managed API above the QCall, never at the QCall itself (`Thread.Yield`,
  `Thread.SpinWait(int)`, `Monitor.Wait(object, int)`, `Pulse`, `PulseAll`).
- Fatal main-thread unhandled exceptions.

Out:
- Thread-pool work, timers and `Task` scheduling. `DotnetRs.Task`
  (`dotnet-assemblies/src/support/Task.cs`, 144 lines) is a single-threaded shim, so
  `Task.Wait`/`.Result` throw "Task has not completed" instead of blocking; deciding whether to grow
  it or use CoreLib's `Task` is the Task-integration work in the README.
- `Thread.Abort`/`Interrupt`, thread priorities, apartment states, `ThreadStatic` identity.
- Making `Monitor.Wait` available in ST beyond what is stated under Design.
- Running `finally` blocks on an unhandled exception: the CLR does not run them either.

## Design

Reuse the slice-5 condition-wait protocol (probe, register, recheck; driver-owned registrations;
activation-owned `Instant` deadlines; waiting outside the mutation with GC-stop participation).
It already covers `Monitor.Enter` and `WaitForPendingFinalizers`. `Sleep` and `Wait` are two more
consumers. Add a facade operation only when a slice's consumer needs it.

**Sleep** is a timed wait with no condition: the activation parks until its deadline, or until
abort. A wake never grants anything. `Sleep(0)` yields once, `Sleep(Timeout.Infinite)` parks until
abort, and other negative values throw `ArgumentOutOfRangeException`. `Yield` returns a bare
yield. `SpinWait(n)` is a bounded yield, not a busy loop.

**Thread identity.** Each managed thread owns one `Thread` object, created lazily for the entry
thread and recorded at `Start` for workers (`ManagedThreadRecord`, `context_ops.rs`). The record
already maps the `Thread` object to its completion handle, so it is the natural home for the managed
id, `IsBackground` and the start argument. `get_CurrentThread` returns the recorded object.
`ManagedThreadId` is unique per thread, stable for its lifetime, and distinct from the OS thread id.
`Start(object)` stores the argument, and `ThreadStart`/`ParameterizedThreadStart` targets with an
instance (closures) are invoked through the ordinary delegate path used for `Invoke`, not rejected.

**Monitor.Wait/Pulse.** `SyncBlock` has an owner, a recursion count and a release epoch; it has no
waiter list. Add one: `Wait` records a waiter, releases the monitor completely (remembering the
recursion count), parks on the condition-wait protocol, and on wake or timeout reacquires through
the existing `monitor_enter` path and restores the count. `Pulse` marks one waiter ready and
`PulseAll` all of them. `Wait`, `Pulse` and `PulseAll` by a non-owner throw
`SynchronizationLockException`. A waiter that times out removes itself, and a `Pulse` never
selects a waiter that has already left. In ST there is no second thread to pulse, so `Wait(timeout)`
sleeps until the timeout and returns `false`, and `Wait` with an infinite timeout throws an
internal deadlock error instead of hanging.

**Background threads.** `wait_for_foreground_workers` already tracks workers through the
registry. Add the background flag to the registry entry and exclude background workers from the
quiescence predicate. On exit, signal them through the existing abort signal and join them at
shutdown, so they stop at their next safepoint.

**Fatal entry exception.** On the entry executor's `Threw` result, record it through
`report_worker_exception` (rename to the neutral `report_fatal_exception`) before `run` waits, so
workers stop at their next safepoint and the first exception wins.

## Slices

1. **Fatal entry-thread exception.** Fixture: the `lock`-holding `Main` above, with a worker that
   would block; the process must exit with the unhandled-exception code. Also an idle sleeping
   worker. Compare stdout and termination with the CLR where the harness can.
2. **`Yield`, `SpinWait`, `Sleep`.** Fixtures: `Yield`/`SpinWait` (ST and MT); `Sleep` as a timed
   park, and the sleeping-worker-versus-`GC.Collect` fixture with a tight but not flaky bound;
   `Sleep(Timeout.Infinite)` ended by abort. Deterministic tests for deadline expiry, abort and a
   GC stop while parked, using the real executor path, as in slice 5.
3. **Thread identity and start shapes.** Fixtures for `CurrentThread` identity on the entry thread
   and on a worker, `ManagedThreadId` uniqueness, `Start(object)`, a closure target, and
   `Join(int)` timing out and succeeding. Delete the instance-target rejection.
4. **`IsBackground`.** Fixtures: a background worker that never returns does not keep the process
   alive; setting it after `Start`; the default for a new thread is foreground. Update the
   foreground-wait documentation in `THREADING_AND_SYNCHRONIZATION.md`.
5. **`Monitor.Wait`/`Pulse`/`PulseAll`.** Fixtures: the producer/consumer pulse above, a timed wait
   that returns `false`, `PulseAll` waking several waiters, a non-owner `Wait` throwing, a
   recursive lock restored after `Wait`, and a `Pulse` with no waiters being lost. Deterministic
   schedule tests for pulse before registration, between registration and recheck, and after
   parking, plus timeout racing a pulse.

## Done when

- Every fixture above passes in the configurations it applies to and matches the CLR where the
  differential harness can compare it.
- `Thread.Yield`, `SpinWait`, `Monitor.Wait` and `Pulse` no longer reach the `QCall` library, and
  `intrinsic_thread_sleep` no longer calls `std::thread::sleep` in the mutation.
- In both configurations: the `threading` and `exceptions` selectors, `cargo test -p dotnet-vm
  --lib`, the whole `integration_tests` binary, and clippy with `-D warnings` for the touched crates.
  Plan 11's follow-up explains why the selectors alone are not enough.
- The README's "Managed threading falls through to QCalls", "`Thread.IsBackground`" and the
  `Monitor.Wait`, `Pulse`, `Thread` closure-target parts of "Other missing or incorrect runtime
  behavior" entries are removed.

## Ask the maintainer only if

- A slice needs a scheduler mechanism plan 08 does not have (for example, a parked activation that
  must be woken from another thread other than through the condition-wait registrations).
- `Monitor.Wait` in ST needs a behavior other than timeout-then-`false` or a deadlock error.
- The `ManagedThreadRecord` cannot carry the new state without a second registry.
