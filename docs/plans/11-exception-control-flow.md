# Exception control flow: handlers inside handlers, unhandled worker exceptions

**Status:** Complete
**Outcome:** A `try`/`catch`/`finally` or `lock` inside a `finally` runs and returns control to the
enclosing `finally` (with or without an exception in flight), and an exception unhandled on a
managed worker thread terminates the process the way an unhandled exception on the main thread
does. Fixtures for both match CLR behavior in both configurations.

## What was fixed

**Handlers inside handlers.** A same-frame `leave` from a protected region inside a running
`finally` overwrote `ExceptionState::ExecutingHandler`, and a `throw` inside a handler never saved
it, so the outer `endfinally` failed with "endfinally called outside of handler". The executing
handler's unwind state is now pushed onto `VmContinuation::HandlerUnwinds` (as
`SavedHandlerUnwind`) whenever a nested unwind or throw starts in the same frame; `endfinally` and
instruction-target completion pop the matching entry, and an unwind that exits a handler (a new
exception or `ret`) discards that handler's entries (`discard_handler_unwinds_from`). Pushing
unconditionally is safe because no valid program leaves or returns from a `finally`/`fault`
(ECMA-335 §III.3.46, §III.3.35, §I.12.4.2.8.2.8).

**Fatal worker exceptions.** The first unhandled worker exception is recorded in
`SharedGlobalState`, executors stop at their next safepoint, and the entry executor returns it as
the main-thread unhandled path does; `Thread.Join` no longer rethrows it. See
[Managed worker lifecycle](../THREADING_AND_SYNCHRONIZATION.md#managed-worker-lifecycle-and-process-exit).

**Exceptions escaping a filter.** The search for an exception thrown inside a filter now stops at
the filter's owning frame (only called methods can hold handlers for it, §I.12.4.2.7), so the
original exception keeps its identity.

Out of scope: `AppDomain.UnhandledException` and `TaskScheduler.UnobservedTaskException` hooks.

## Remaining limit

`Executor::run` waits for workers in `Finalizing`, so a retired owner's finalizer that never returns
blocks `run`; the CLR does not wait for finalizers at exit. `Executor::run_entrypoint` returns when
the entry method does (`shutdown_waits_for_finalizing_worker_and_joins_workers_it_starts` uses it).
Tracked in the plans README, "Not yet planned".

## Checks

- `crates/dotnet-cli/tests/fixtures/exceptions/handlers_inside_handlers_42.cs`, `nested_finally_42.cs`
  and `filter_throws_keeps_original_42.cs` are CLR differential fixtures; the `fault` shapes have
  VM-level tests.
- `threading_fatal_exceptions_match_dotnet` (`crates/dotnet-cli/tests/integration_tests_impl/diff_harness.rs`)
  compares the fatal worker cases (`tests/debug_fixtures/fatal_*.cs`) with the CLR's exit status
  and stdout.
- `native_activations_follow_nested_filter_segments` covers native calls across nested filters.

Selectors miss non-fixture tests: run `cargo test -p dotnet-vm --lib` and the whole
`integration_tests` binary (no selector) in both configurations, not only the `exceptions` and
`threading` fixture selectors.
