# Exception Handling

This document describes the structured exception handling (SEH) system in `dotnet-rs`, which implements the ECMA-335 exception model.

## Overview

Exception handling is implemented as a **state machine** in `crates/dotnet-vm/src/exceptions.rs` (~760 lines). The system handles `try`/`catch`/`finally`/`fault`/`filter` blocks and coordinates with the call stack for two-pass exception processing (search phase, then unwind phase).

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

<!-- TODO: Document the detailed flow of each method -->

- **`handle_exception`**: Entry point called by the dispatch loop when `ExceptionState` is not `None`. Routes to the appropriate phase method.
- **`begin_throwing`**: Initializes the search state. Sets up the handler cursor starting from the current frame's handlers.
- **`search_for_handler`**: Iterates through handlers across frames. For `catch` handlers, checks type compatibility. For `filter` handlers, transitions to `Filtering` state. If no handler is found in a frame, pops the frame and continues searching up the call stack.
- **`unwind`**: Executes finally/fault handlers between throw site and target. When all intermediate handlers have run, transitions to `ExecutingHandler`.

## Non-Obvious Connections

<!-- TODO: Expand each of these with code references and examples -->

### Integration with Dispatch Loop (`dispatch/mod.rs`)
- `ExecutionEngine::step` checks `ExceptionState` before normal instruction dispatch. If an exception is active, it calls `handle_exception` instead of fetching the next instruction.
- The `step_normal` method catches `StepResult::Throw` from instruction handlers and transitions to `Throwing` state.

### Integration with Call Stack (`stack/frames.rs`, `stack/context.rs`)
- `ProtectedSection` data is stored per-frame (parsed from method metadata during frame setup).
- Frame unwinding during exception handling must properly clean up the evaluation stack — `BasePointer` tracks where each frame's stack values begin.
- `VesContext::unwind_frame` is called during the unwind phase to pop frames.

### Filter Execution
- Filters are unusual: they execute arbitrary user CIL code *during* the exception search phase. The VM transitions to `Filtering` state, resumes normal dispatch to run the filter code, then reads the `endfilter` result to decide whether to handle the exception.
- This means the dispatch loop must handle re-entrant exception states.

### `leave` Instruction
- The `leave` instruction (used to exit try/catch blocks) triggers the unwind mechanism to run finally blocks, even though no exception is active. It uses `UnwindTarget::Instruction` instead of `UnwindTarget::Handler`.

### Interaction with Delegates
- Multicast delegate invocation in `ExecutionEngine::handle_multicast_step` must handle exceptions thrown by individual delegate targets, as each target runs as a separate method call.

## Notes for Future Documentation

- [ ] Add sequence diagrams for the two-pass exception model
- [ ] Document the relationship between `ExceptionState` transitions and `StepResult` variants
- [ ] Explain how `rethrow` preserves stack traces vs `throw` resetting them
- [ ] Document edge cases: nested exceptions, exceptions in finally blocks, exceptions in filters
- [ ] Detail the `parse` function that converts dotnetdll metadata to `ProtectedSection`/`Handler`
