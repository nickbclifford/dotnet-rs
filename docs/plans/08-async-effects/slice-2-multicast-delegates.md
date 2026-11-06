# Plan 08 slice 2: multicast delegates through transparent calls

**Status:** Notes. Fill in Done-when and open questions before starting.

## Consumer

`dotnet-intrinsics-delegates/src/invoke.rs`: multicast invocation keeps target array, index and
arguments in `MulticastState` on a frame (`dotnet-vm-data/src/stack.rs`, `StackFrame::multicast_state`).
Single-target dispatch stays synchronous.

## Introduced here (not in slice 1)

- Activation stack with monotonic `NativeId`, and a native return destination on managed
  frames (proposed: a new frame field next to `frame_continuation`).
- Single-slot mailbox and reply generations. One generation per effect, assigned on first
  poll. Mismatched replies are internal errors.
- A logical native boundary in exception search. Insert the check before the frame-zero
  fallback in `dotnet-exceptions/src/lib.rs`. Unhandled handling there clears every frame and
  must not run past a native boundary.
- Finalizer segment isolation. The finalizer runs as a separate managed segment with its own
  ambient exception context and never pushes frames onto a parked caller. Replaces the slice-1
  deferral. This changes finalizer timing relative to parked natives.
- Native `call` from inside an active filter or handler. Requires plan 07. Activations move
  with their suspended segment and are not polled while it is suspended.

## Carried from the archive (`archive/async-effects-2026-10`, DESIGN.md)

- Transparent search skips the native boundary without resuming Rust. If the handler lies
  outside, child finally/fault blocks run, and the future is dropped when unwind crosses the
  boundary. If the handler lies inside, the parent stays alive.
- A tail call or `jmp` in a managed child must keep its return-to-native destination and
  boundary. Add dedicated tests.
- Index-based `HandlerAddress` is valid only on the active stack. Use stable IDs for native
  linkage.
- Bound inline effect completions per engine entry (archive: 32), separate from the 32 access
  grants.
- Stack dumps show one synthetic frame per native boundary (intrinsic name, native ID, IL
  offset), never Rust poll frames.

## Open questions

- Is `try_call` (capturing) needed by any slice-2–5 consumer? If not, defer it.
- Where does the native return destination live: a `StackFrame` field or a separate side
  table keyed by frame identity?

## Done when (draft)

- Multicast fixtures pass. `MulticastState` is deleted.
- CLR-compared fixtures: a throwing target with outer filters (filter order), a target that
  catches internally, a nested multicast inside a target, multicast from inside a filter.
