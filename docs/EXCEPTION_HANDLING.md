# Exception Handling

This document describes the structured exception handling (SEH) system in `dotnet-rs`, which implements the ECMA-335 exception model.

## Overview

Exception handling is split across three crates:

- **`dotnet-vm-ops`** (`crates/dotnet-vm-ops/src/lib.rs`, re-exporting `crates/dotnet-vm-data/src/exceptions.rs`): Exception data types — `ExceptionState`, `ProtectedSection`, `Handler`, `HandlerKind`, `HandlerAddress`, `ManagedException`, `SearchState`, `FilterState`, `UnwindState`, `UnwindTarget`.
- **`dotnet-exceptions`** (`crates/dotnet-exceptions/src/lib.rs`, ~584 lines): The `ExceptionHandlingSystem` with the two-pass search/unwind state machine. Depends on `dotnet-vm-ops` for base traits and types.
- **`dotnet-vm`** (`crates/dotnet-vm/src/dispatch/mod.rs`, `crates/dotnet-vm/src/stack/context.rs`, `crates/dotnet-vm/src/stack/exception_ops_impl.rs`): Runtime integration points that drive `ExceptionState` transitions and invoke `dotnet-exceptions`.

The system handles `try`/`catch`/`finally`/`fault`/`filter` blocks and coordinates with the call stack for two-pass exception processing (search phase, then unwind phase).

## State Machine

The core type is `ExceptionState`, an enum with these transitions:

```
None → Throwing → Searching → Filtering (optional) → Unwinding → ExecutingHandler → None
```

### States

- **`None`**: No active exception.
- **`Throwing(ObjectRef, bool)`**: An exception has been thrown. The `bool` indicates whether to preserve the existing stack trace (for `rethrow`).
- **`Searching(SearchState)`**: First pass — walking handlers in the current and parent frames to find a matching `catch` or `filter`. Tracks a `HandlerAddress` cursor.
- **`Filtering(FilterState)`**: Executing a filter clause to decide if it handles the exception. The filter runs user code and produces an `endfilter` result.
- **`Unwinding(UnwindState)`**: Second pass — executing `finally`/`fault` blocks encountered between the throw site and the target handler. Tracks an `UnwindTarget`.
- **`ExecutingHandler(UnwindState)`**: The matched handler is running (catch block body).

### Key Types

- **`HandlerAddress`**: Cursor into the handler search space — tracks frame index and handler index.
- **`UnwindTarget`**: Where unwinding should stop — either a `Handler(HandlerAddress)` or an `Instruction(usize)` (for `leave` targets).
- **`ProtectedSection`**: A try region with its associated handlers, parsed from metadata.
- **`Handler`**: A single handler block with offset, length, and `HandlerKind`.
- **`HandlerKind`**: `Catch(ConcreteType)`, `Filter { clause_offset }`, `Finally`, `Fault`.
- **`ManagedException`**: Extracted exception info (type name, message, stack trace) for display.

## Key Methods on `ExceptionHandlingSystem`

- **`handle_exception`** (`crates/dotnet-exceptions/src/lib.rs`): State dispatcher for `ExceptionState`. Called from `VesContext::handle_exception` in `crates/dotnet-vm/src/stack/context.rs`, which is invoked by `ExecutionEngine::step`/`run` in `crates/dotnet-vm/src/dispatch/mod.rs`.
- **`begin_throwing`** (`crates/dotnet-exceptions/src/lib.rs`): Captures stack trace text (including intrinsic context), preserves existing traces for `rethrow`, resets suspended state when appropriate, and seeds `Searching(SearchState)` with a `HandlerAddress` cursor.
- **`search_for_handler`** (`crates/dotnet-exceptions/src/lib.rs`): Scans `frame.state.info_handle.exceptions` across frames, transitions to `Unwinding` for matching catches, transitions to `Filtering` for filter clauses (suspending stacks), and returns `StepResult::MethodThrew` when no handler is found.
- **`unwind`** (`crates/dotnet-exceptions/src/lib.rs`): Executes `finally`/`fault` handlers while exiting protected sections, pops frames via `ctx.unwind_frame()`, and finally jumps to either the handler start (`UnwindTarget::Handler`) or a `leave` target (`UnwindTarget::Instruction`).

## Non-Obvious Connections

### Integration with Dispatch Loop (`dispatch/mod.rs`)
- `ExecutionEngine::step` (`crates/dotnet-vm/src/dispatch/mod.rs`) checks `ExceptionState` before normal instruction dispatch. If an exception is active, it routes to `VesContext::handle_exception()`.
- Instruction handlers signal exceptions with `StepResult::Exception` (not `StepResult::Throw`). The throw/rethrow opcodes in `crates/dotnet-vm/src/instructions/exceptions.rs` call `ExceptionOps`, and `VesContext` updates `ExceptionState::Throwing` in `crates/dotnet-vm/src/stack/exception_ops_impl.rs`.

### Integration with Call Stack
- `ProtectedSection` data is parsed by `dotnet_exceptions::parse` during method-info construction in `crates/dotnet-vm/src/lib.rs` (`build_method_info`) and stored on each frame in `MethodInfo::exceptions`.
- `StackFrame`, `FrameStack`, and `BasePointer` are defined in `dotnet-vm-data/src/stack.rs` and re-exported by `dotnet-vm-ops`.
- Frame unwinding during exception handling must properly clean up the evaluation stack — `BasePointer` tracks where each frame's stack values begin.
- `VesContext::unwind_frame` (`crates/dotnet-vm/src/stack/context.rs`) is called during unwind to pop frames, clear stack slots, and wrap `.cctor` failures as `TypeInitializationException`.

### Filter Execution
- In `search_for_handler` (`crates/dotnet-exceptions/src/lib.rs`), filter handlers transition to `ExceptionState::Filtering`, suspend higher frames/stacks (`suspend_above`), set the filter IP, and push the exception object as filter input.
- The `endfilter` opcode (`crates/dotnet-vm/src/instructions/exceptions.rs`) calls `ExceptionOps::endfilter` (`crates/dotnet-vm/src/stack/exception_ops_impl.rs`), which restores suspended state and continues either unwinding (`result == 1`) or searching (`result == 0`).
- This means the dispatch loop must handle re-entrant exception states.

### `leave` Instruction
- The `leave` instruction (used to exit try/catch blocks) triggers the unwind mechanism to run finally blocks, even though no exception is active. It uses `UnwindTarget::Instruction` instead of `UnwindTarget::Handler`.

### Rethrow Semantics
- The `rethrow` instruction (ECMA-335 §III.4.24) is used within a catch handler to re-propagate the caught exception. Unlike the `throw` instruction, which resets the exception's stack trace to the current execution point, `rethrow` MUST preserve the original stack trace of the exception object.
- In `dotnet-rs`, this is handled by the `bool` parameter in `ExceptionState::Throwing(ObjectRef, bool)`.
- When `throw` is called, it passes `false`, causing `begin_throwing` to generate and write a new stack trace string to the `_stackTraceString` field of the exception object.
- When `rethrow` is called, it passes `true`. `begin_throwing` will then check if the exception object already contains a stack trace. If it does, it skips generating a new one, thereby preserving the original trace from the initial throw site.

### Interaction with Delegates
- Multicast delegate invocation in `ExecutionEngine::handle_multicast_step` must handle exceptions thrown by individual delegate targets, as each target runs as a separate method call.

## Implementation Details

### Rethrow Stack Trace Preservation
- **`crates/dotnet-vm-data/src/exceptions.rs`**: `ExceptionState::Throwing` includes a `bool` flag for trace preservation.
- **`crates/dotnet-vm/src/stack/exception_ops_impl.rs`**: `rethrow()` sets this flag to `true`, while `throw()` sets it to `false`.
- **`crates/dotnet-exceptions/src/lib.rs`**: `begin_throwing()` reads this flag and optionally skips stack trace generation if an existing trace is present and preservation is requested.

## Notes for Future Documentation

- [ ] Add sequence diagrams for the two-pass exception model
- [ ] Document the relationship between `ExceptionState` transitions and `StepResult` variants
- [x] Explain how `rethrow` preserves stack traces vs `throw` resetting them
- [ ] Document edge cases: nested exceptions, exceptions in finally blocks, exceptions in filters
- [ ] Detail the `parse` function that converts dotnetdll metadata to `ProtectedSection`/`Handler`
