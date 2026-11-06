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

## Verified facts and hazards (HEAD 069c8294, read from code, not reproduced)

- Flow today: `invoke_delegate` (`invoke.rs:17-115`) pushes a dummy frame and installs
  `MulticastState`; `step_normal` intercepts it (`dispatch/mod.rs:135-137`);
  `handle_multicast_step` (`:334-407`) pops the previous return, pushes the next target and
  dispatches it; `handle_return` skips the IP increment for a caller with multicast state
  (`stack/context.rs:321-325`). Tail calls are refused for such frames
  (`call_ops_impl.rs:1029`).
- **`.cctor` in a target corrupts multicast.** `dispatch_method` runs `.cctor` init for types
  without `beforefieldinit` (`call_ops_impl.rs:557-571`) and returns before dispatching the
  target. The `.cctor` return is treated as auto-invoked (`context.rs:295-298`), so the next
  `handle_multicast_step` pops the pushed target arguments as a "return value" and skips that
  target. Add a regression fixture; deleting `MulticastState` removes the cause.
- **Finalizers push frames onto the live stack** (`context_ops.rs:1225-1235`) and do not check
  `exception_mode`, so one can be pushed while the thread is filtering or in a handler. An
  exception from a finalizer is searched through the interrupted frames to frame 0. Segment
  isolation fixes both.
- **`jmp`** (`call_ops_impl.rs:1147-1257`) carries only `constructor_result`. It must carry the
  native return destination as well. `should_honor_tail_call` (`:1017-1064`) already refuses
  several frame kinds; add the native-return case.
- Existing fixtures: `delegates/delegate_multicast_*`, `delegate_combine_remove_42`,
  `exceptions/multicast_exception_42` (target A throws, Main catches, B must not run). None
  is CLR-diffed (`tests/integration_tests_impl/diff_harness.rs:78-155`). No fixture covers a
  filter, an internal catch, nested multicast or events.
- Transparent `call` suffices; no consumer in this slice needs `try_call`.

## Open questions

- Where does the native return destination live: a `StackFrame` field or a separate side
  table keyed by frame identity?

## Done when (draft)

- Multicast fixtures pass. `MulticastState` is deleted.
- CLR-compared fixtures: a throwing target with outer filters (filter order), a target that
  catches internally, a nested multicast inside a target, multicast from inside a filter.
