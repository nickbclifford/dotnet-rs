# Retire legacy continuations and move suspending intrinsics to effects

**Status:** Complete
**Outcome:** Suspending intrinsic routes use native effects; no intrinsic re-executes its
instruction after a safepoint yield. The multicast continuation still retained at this plan's
completion was removed by [plan 26](26-thread-lifecycle-and-multicast-fallback.md).

## What was retired

Before this plan, intrinsics that crossed a safepoint returned `StepResult::Yield` and re-ran
their instruction, or parked state on the caller's frame. Each mechanism and its outcome:

| Mechanism | Outcome |
|---|---|
| Yield-retry intrinsics: `Thread.Join()`/`Join(int)`, `LowLevelLock.Acquire`, `Monitor.TryEnter_FastPath` family, `GC.Collect` ×3 | Native bodies. Join waits on worker completion with an absolute deadline that survives GC stops; `GC.Collect` requests a full collection and yields until it is serviced (the request stays pending if the coordinator is busy). |
| Reflection `InvokeReturn` continuation, `FrameReturnAction`, the per-instruction `reflection_pending` check | Deleted; every reflection call takes the native body. Byref-like parameters, returns and `Activator` targets throw the CLR's `NotSupportedException` before the target runs. |
| `MulticastState` continuation | Kept for excluded shapes at this plan's completion; deleted by plan 26. |
| Monitor byref fallbacks and `StackFrame::monitor_retry` | Deleted. `Unmanaged` `bool&` origins are admitted (unchecked guest trust, `F10.RawMemoryAccessValid`); a `Transient` origin raises `InvalidProgramException` before operands are committed. The only `Transient` producer, `ldflda` on a value on the evaluation stack, is invalid IL (ECMA-335 §III.4.11) and the CLR rejects it. |
| Public `ExecutionEngine` dispatch APIs and the `ExecutionEngineApi` mode | Deleted (no production callers). Intrinsic-constructor `newobj` dispatches in `NativeDispatchMode::Constructor`, so async constructors can suspend; a value-type receiver is published only on success. |
| Legacy static-initializer triggers (`newobj`, `prepare_method_initialization`, `RunClassConstructor`, the native-call preflight), frame `initializer_claim`, `wait_for_init`, late wrapping in `unwind_frame` | One trigger, `VesContext::initialize_type_then`, resuming with `InitializerResume::Retry` (`newobj`) or `InitializerResume::Dispatch` (re-dispatch from the operand stack, which delegates need because dispatch already rewrote it). Outer filters see only `TypeInitializationException`, as on the CLR. |
| Bulk loops that restarted from offset 0 after a yield | See below. |

`.cctor` dispatch and the span-equality slow path still use non-inline dispatch; they run managed
bodies, never intrinsics.

### Bulk operations

Restarting an overlapping copy from zero after a yield was not idempotent. Under
`multithreading`, `memmove_overlap_gc_42` reproduced it (`OwnerUnavailable`) before the change.

- `Buffer.Memmove`/`SpanHelpers.Memmove`, `SpanHelpers.ClearWithoutReferences` and byte-compatible
  `SpanHelpers.Fill<T>` have async bodies (`memmove_async`, `clear_async`, `fill_async` in
  `crates/dotnet-intrinsics-unsafe/src/buffer.rs`) that root both operands as descriptors and copy
  1 MiB chunks, re-resolving and range-checking each time; progress survives every yield.
  Operations of at most one chunk keep a synchronous fast path.
- Large `String.Equals`, `GetHashCodeOrdinalIgnoreCase`, `IsNullOrWhiteSpace` and the
  repeated-character and char-array constructors keep roots and progress in native bodies.
- `cpblk`, `initblk`, `Unsafe.CopyBlock*`, `Unsafe.InitBlock*`, non-byte `SpanHelpers.Fill<T>` and
  the remaining string and span loops run to completion without polling (accepted: the CLR's
  block-copy helpers do not poll either; the delay is bounded by one linear pass, at most 4 GiB
  for the `uint` forms). No string, span or unsafe intrinsic returns `Yield` now.
- Safepoints taken before any work starts remain in `newarr` and the reflection
  `MakeGenericType`/`GetGenericArguments` handlers, which restart harmlessly.

## Accepted costs

- Removing the per-instruction reflection check let LLVM inline `step_normal` into the run loop;
  under `multithreading` that made `dispatch` about 41% slower through store-to-load forwarding
  stalls. `step_normal` is `#[inline(never)]`, restoring the baseline. With it, `dispatch` changed
  −0.77% (ST) and −2.24% (MT).
- Bulk-family benchmarks stayed within about 1.6% except MT `string`, which moved +7.18% and later
  −6.46% with unchanged instruction counts: code placement of the hot `IsNullOrWhiteSpace` loop
  (see "String benchmark is code-placement sensitive" in the plans README).

## Remaining limits

- Static storage keys canonicalize through aliases recorded by `init`, so cross-assembly `TypeRef`
  versus `TypeDef` unification depends on `init` having seen the spelling first (untested).
- `original_ip`/`original_stack_height` are still saved on every instruction and around nested
  dispatch but may no longer be read; removing them touches the layout-sensitive hot path and
  needs a benchmark.
- The reflection `.cctor` filter fixture cannot observe filter timing (the exception is rethrown
  after `TargetInvocationException` unwrapping), so it checks only the final type.

Both open items are in the plans README, "Not yet planned".

## Checks

CLR differential fixtures: `statics/cctor_filter_{newobj,call,delegate,delegate_multicast,reflection,run_class_constructor}_42.cs`,
`cctor_success_triggers_42.cs`, `cctor_recursive_triggers_42.cs`,
`cctor_wait_triggers_{success,failure}_42.cs` (MT), `reflection/invoke_byref_like_42.cs`,
`unsafe/memmove_overlap_gc_42.cs`, `unsafe/bulk_fill_gc_42.cs`, `strings/equality_gc_42.cs` (MT,
with a second thread forcing collections), `strings/string_char_repeat_42.cs` and
`unsafe/span_clear_42.cs`. Unit tests: `join_timeout_survives_collection`,
`managed_join_resolves_completion_published_after_start`,
`collection_request_retries_when_coordinator_is_busy`,
`native_newobj_completes_string_and_value_receiver_after_gc`,
`trigger_initializer_host_error_releases_waiter` and the registry guard
`blocking_intrinsics_have_no_sync_fallback`.
