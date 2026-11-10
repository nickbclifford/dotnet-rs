# Exception control flow: handlers inside handlers, unhandled worker exceptions

**Status:** In progress
**Outcome:** A `try`/`catch`/`finally` or `lock` inside a `finally` runs and returns control to the
enclosing `finally` (with or without an exception in flight), and an exception unhandled on a
managed worker thread terminates the process the way an unhandled exception on the main thread
does. Fixtures for both match CLR behavior in both configurations.

## Problem

Reproduced on `main` (2026-11-09, `--no-default-features`) with throwaway fixtures; the
sources are in the shapes below and are not in the tree.

**1. Protected regions inside a `finally` in the same frame lose the handler.**
All of these fail with `InvalidCil("endfinally called outside of handler, state: None")`:

- `try { r += 1; } finally { try { r += 2; } finally { r += 4; } }`. No exception is involved.
- `try { throw A; } finally { try { r += 1; } finally { r += 2; } }`.
- `try { throw A; } finally { lock (O) { r += 1; } }`. The C# `lock` is a `try/finally`.
- `try { throw A; } finally { try { throw B; } catch (B) { r += 1; } r += 2; }`.

These pass today, which bounds the defect:

- a `finally` that *calls* a method containing `try/finally` (callee frame);
- a `finally` inside a `catch`;
- an exception thrown by a `finally` replacing the pending one (`nested_finally_42.cs`).

Cause: while a `finally` runs the mode is `ExceptionState::ExecutingHandler(state)`, and `state`
holds the unwind to resume at `endfinally`. `leave` saves it
(`push_handler_unwind`, `dotnet-vm/src/stack/exception_ops_impl.rs:leave`) only when
`state.cursor.frame_index < frame_index`, that is, from a deeper frame. A same-frame `leave`
overwrites the mode with `Unwinding`, nothing is pushed, and the later outer `endfinally` finds
`None`. A `throw` while `ExecutingHandler` is active (case 4) never saves the state at all;
`push_handler_unwind` has no other caller. `dotnet-exceptions/src/lib.rs` pops the saved state
only for `UnwindTarget::Instruction` completions.

`lock` and `using` in cleanup code are common, so this is the highest-value item here.

**2. An unhandled exception on a worker is reported through `Thread.Join`.**
The worker's executor returns `ExecutorResult::Threw`; `threading_join_managed_thread`
(`dotnet-vm/src/stack/context_ops.rs`) turns it into
`InvalidOperationException("Managed ThreadStart failed: …")`. The CLR terminates the whole
process at the throw. Reproduced under `multithreading`: a worker that throws, joined inside
`try/catch (InvalidOperationException)`, makes the VM exit 42; CLR aborts (exit 134). A worker
that is never joined presumably loses its exception entirely. That was not reproduced.

The same behavior is a prerequisite for thread-pool work items (see "Task and thread-pool
integration" in the README), where a swallowed exception would hide failures.

**3. Exceptions escaping a filter: already correct.** The README previously listed this as a
defect. `filter_throws_keeps_original` (a filter that throws `ArgumentException`, with an outer
`catch (InvalidOperationException)` that must still see the original) passes, and
`nested_filter_throws_42.cs` covers the nested shape. The `nested_filter` search state
implements §III.3.34 / §I.12.4.2.7. Only the regression fixture is missing.

## Scope

In:
- Same-frame nested `try/catch/finally/fault` inside a `finally` or `fault` block, in both the
  normal and the exception-pending case, including `lock`.
- A throw inside a handler that is caught inside that handler, and one that escapes it. The
  escaping exception replaces the pending one, and the saved state of every exited handler is
  discarded.
- Worker unhandled exceptions: process termination, message, exit code, and no continuation of
  other managed threads.
- A regression fixture for filter-exception swallowing.

Out:
- `Thread.IsBackground`, thread-pool workers and `Task` (README "Task and thread-pool
  integration"). This plan only guarantees that an unhandled worker exception is fatal.
- `AppDomain.UnhandledException` / `TaskScheduler.UnobservedTaskException` hooks.
- Filter timing and two-pass semantics (plan 07).
- A `leave` that exits a `finally` or `fault` block, or a `ret` inside one. No valid program
  produces either: §III.3.46 ("It is not valid to use a leave instruction to exit a finally
  block"), §III.3.35 ("it is not valid to ... execute a ret or leave instruction within a
  finally (or fault) block") and §I.12.4.2.8.2.8 leave source rule 1 (a `leave` whose source is
  in a finally, fault or filter must target the same block). So while `ExecutingHandler` is
  active in the current frame, every `leave` stays inside that handler, and slice 1 may push
  unconditionally in that case. Cite these sections in the code.
- Protected regions inside a filter: §I.12.4.2.7 forbids nesting one inside a filter. Filters
  reach exception handling only through the methods they call, which is the callee-frame case
  that already works.

## Design

**Slice 1.** Make "a handler is executing" a stack, not a single mode. Whenever a nested
unwind or throw begins while `ExecutingHandler(state)` is active in the same frame, push `state`
(the `<` guard in `leave` becomes `<=`, which is safe because of the spec rules under Scope; the
throw path needs the same push). Entries are tied to the handler they belong to (frame index and handler range).
`endfinally` and the `Instruction`-target completion pop the matching entry. Unwinding that exits
a handler, whether by a new exception or by `ret`, must drop that handler's entries rather than
resume them. Without that rule the escape case would leave a stale entry that resurrects the
replaced exception. `VmContinuation::HandlerUnwinds` already exists and is traced, so reuse it.

**Slice 2.** Route a worker's `ExecutorResult::Threw` through a single process-fatal path:
record the first such exception in `SharedGlobalState`, stop every executor at its next
safepoint, and make the entry executor's `run` return the same `Threw` the main-thread
unhandled path produces. The CLI then prints it and exits with the existing unhandled-exception
code. The worker registry already has abort and shutdown signals (`WorkerRegistry::shutdown`,
`workers.rs`); use them rather than adding a new stop mechanism. `Thread.Join` then never
observes `Threw`. Keep the `InvalidOperationException` path only for `Error` results.

## Slices

1. **Handlers inside handlers (implemented).** Add fixtures for the four failing shapes above, plus the three
   passing ones as regression guards, plus escape and `fault`-in-`finally` variants. Compare
   with CLR (the exit-code `diff_test!` list in `tests/integration_tests_impl/diff_harness.rs`).
   Extend the `dotnet-exceptions` / `frame_alloc::` tests to cover the saved-state stack,
   including discard on escape. The C# fixture and generated `fault` IL match the CLR in both
   configurations; the fault cases also have VM-level tests.
2. **Fatal worker exceptions.** Fixtures (`multithreading` only): joined worker throws; unjoined
   worker throws while main waits; worker throws while another worker is mid-computation. The
   exit status and the stdout written before the throw are compared with CLR where the harness
   can (subprocess comparison: CLR 134 versus the CLI's exit code; see the rejected-candidate
   note in `diff_harness.rs`). Remove the `Thread.Join` rethrow of `Threw`, and update
   `threading_join_managed_thread`'s tests in `native_substring_tests.rs` if they depend on it.
3. **Filter regression fixture.** Add `filter_throws_keeps_original_42.cs` (shape in problem 3)
   as an exit-code differential test.

## Done when

- The slice 1 fixtures pass in both configurations and match CLR; the INVENTORY note for
  handler state, if one exists, is updated.
- Slice 2 fixtures pass under `multithreading`; an unhandled worker exception can no longer be
  caught through `Thread.Join`.
- Exception fixture selector `exceptions` and `threading` pass in both configurations; clippy
  for `dotnet-exceptions`, `dotnet-vm`, `dotnet-vm-data` and `dotnet-cli` with `-D warnings`.
- The README's "Unhandled worker exceptions" and "Control flow and filters" entries are
  removed.

## Ask the maintainer only if

- CLR output for a slice 1 fixture contradicts the current replace-the-pending-exception
  behavior of `nested_finally_42.cs`.
- Making worker exceptions fatal requires stopping executors somewhere other than a safepoint.
- The handler-state stack needs a representation outside `VmContinuation::HandlerUnwinds`.
