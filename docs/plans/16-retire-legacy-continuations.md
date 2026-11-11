# Retire legacy continuations and move suspending intrinsics to effects

**Status:** In progress
**Outcome:** Every intrinsic that blocks, retries or yields for a GC safepoint is an async
body, and the guest-visible dispatch routes that still push `MulticastState` or
`FrameReturnAction::InvokeReturn` frames use effects. Each legacy mechanism either is deleted
(`grep` finds no user) or remains with a one-line reason in the "Kept" table below, checked by a
test that fails if the reason stops being true.

## Problem

[Plan 13](13-async-intrinsic-api.md) made an async body sufficient for an intrinsic, but only
about 12 bodies exist against about 588 `#[dotnet_intrinsic]` registrations. Most registrations are
leaf operations that never suspend; converting them would add cost and remove nothing. The ones
that matter still hand-roll suspension as `StepResult::Yield` plus re-execution, or park
state on the caller's frame:

| Legacy mechanism | Users today |
|---|---|
| Sync yield-retry intrinsics (re-run the instruction after `StepResult::Yield`) | `Thread.Join()`/`Join(int)` (`thread.rs`, `join_managed_thread` in `context_ops.rs`), `LowLevelLock.Acquire`, `Monitor.TryEnter_FastPath` family (`monitor.rs`), `GC.Collect` x3 (`gc.rs`, `increment_ip` + `Yield`) |
| Bulk loops that yield at `check_gc_safe_point` | string `operations`/`constructors`/`search`, `mem_helpers`, `buffer`, span `equality`, `cpblk`/`initblk` (`instructions/memory.rs`) |
| `MulticastState` on `StackFrame` (+ `handle_multicast_step`, `dispatch/mod.rs`; `call_ops_impl.rs:1090`, `context.rs:464`) | multicast `Invoke` shapes that `try_start_multicast` (`native_effects.rs`) declines; a native multicast body (`multicast_body`) already serves the rest |
| `FrameReturnAction::InvokeReturn` + `apply_frame_return_actions` + `reflection_pending` | reflection `Invoke`/`CreateInstance` fallbacks for shapes `invoke_select`/`activator_select` decline; `run_reflection_call` already serves the rest |
| Test seams that force a fallback (`MULTICAST_FALLBACK_TEST` in `native_effects.rs`, the reflection retry equivalent) | `multicast_fallback_completes_collection_targets`, `reflection_retry_completed_collection_does_not_retry`: they drive `GC.Collect` targets through the legacy continuations because no natural shape of that signature declines |
| `StackFrame::monitor_retry` + `monitor_retry_deadline`/`monitor_clear_retry` | timed byref `Monitor` fallbacks (Transient/Unmanaged `bool&`) and `Thread.Join(int)` |
| `NativeDispatchMode::Other` + `fallback =` registration | the residual callers in plan 13 |
| `prepare_method_initialization`, `newobj` and `RunClassConstructor` initialization | legacy static-init triggers (README "Legacy static-initialization triggers") |

### Declined shapes (why the continuations are still live)

Slice 1 added native-call transport for plain value types and enums, and admitted `Static` byref
origins. The remaining declined shapes are:

- Byref arguments require `Heap`, `FrameHome`, `Static` or MT cross-arena origins, and a target
  type without managed pointers.
- Byref returns and `TypedReference` are declined in both.
- Ref-struct arguments (`Span<T>`, `SpanAction`) carry `ManagedPtr` fields whose origins may be
  `Transient` or `Unmanaged`; `stash` rejects those, so they cannot simply be admitted as values.

Deleting either continuation still requires a decision for the remaining declined shapes.

### Native calls still use legacy initialization

`dispatch_pending_native_call` (`native_effects.rs`) runs `prepare_method_initialization` as a
preflight for every unstaged native call, saving and restoring `original_ip` /
`original_stack_height` around it so the legacy retry machinery does not replay the caller. Every
effect body that calls managed code (multicast, reflection, and anything new in this plan)
therefore depends on the legacy static-init trigger. Static-init removal must convert this
preflight to `ensure_initialized`, not just the three IL triggers.

### Bulk loops restart from zero

The chunked loops keep no progress: `StepResult::Yield` re-executes the call instruction, which
starts the copy again at offset 0. For non-overlapping copies this is wasted work and, under a
steady stream of stop requests, a possible livelock. For **overlapping** `Buffer.Memmove` /
`cpblk` above `MEM_OP_CHUNK_SIZE` (1 MiB, `mem_helpers.rs`, `buffer.rs`) a restart is not
idempotent: the chunks already written have overwritten part of the source. This is only
reachable under `multithreading` (`check_gc_safe_point` reports a requested stop), and has not
been reproduced. A suspending body that keeps its offset in the future fixes both by
construction.

Note: `VesContext::unwind_frame` is **not** legacy plumbing. It is the exception unwinder's frame
pop (`dotnet-exceptions/src/lib.rs`). Only its late `TypeInitializationException` wrapping
(`is_cctor && !capturing_initializer`) and `initializer_claim` failure belong to the legacy
static-init triggers.

## Fallback audit (what can and cannot go)

Checked against the code on `main`. "Reachable" means a valid guest program can reach it.

| Fallback / mechanism | Verdict | Reason |
|---|---|---|
| `Thread.Join` yield loop | **Remove** | No operand-shape problem; the operand is an object and an int. Becomes a body that registers a completion wait. Frees `monitor_retry` from its second user. |
| `LowLevelLock.Acquire`, `TryEnter_FastPath`, other `monitor_enter_safe` + `Yield` users | **Remove** | Same shape as already-migrated `Monitor.Enter(object)`. |
| `GC.Collect` x3 | **Remove** | The `increment_ip` + `Yield` trick is a hand-written effect. Needs an effect that requests a full collect and resumes after the stop. |
| Bulk string/mem/span/buffer loops | **Remove, measured** | Plan 04 measured a large-call cost for native activations. Each family needs a before/after on its benchmark (`substring`, `end_to_end`, `runtime_primitives`); small-input paths keep an inline fast path as Substring does. A fast path registered as a sync `fallback` is also what any remaining `Other` dispatch receives, so it must either handle every input size or `Other` must have no producer for that intrinsic. Overlapping memmove (above) is a correctness reason, not only a cleanup. |
| `cpblk`/`initblk` instruction yields | **Fix, then decide** | These are instructions, not intrinsics, but share `chunked_copy_with_safe_point` and its restart-from-zero overlap hazard. At minimum keep progress across the yield; whether they become effects is decided with the `mem_helpers` family. |
| Multicast `Invoke` (`MulticastState`) | **Remove after struct transport** | The native body admits plain values and static byrefs. Ref-struct arguments, byref returns and `TypedReference` need a decision: admit, or keep the continuation for exactly those with a "Kept" row. The `retrying_initializer` flag goes with the static-init slice. |
| Reflection `InvokeReturn` / `reflection_pending` | **Remove after struct transport** | Wrong receiver type and argument validation still fall back (plan 13's throw helper moves these into the body); byref write-back after a throw is a README deviation to fix in the body. Ambiguous-match "first best match" behaviour is unchanged unless deleting a fallback requires it. Deleting `reflection_pending` also removes the per-instruction pending-reflection check (README "Pending-reflection instruction cost"); measure it. |
| Monitor byref `Transient` / `Unmanaged` fallback | **Verify, then likely keep** | `lock` statements use a local (`FrameHome`), admitted. A `Transient` `bool&` has no obvious producer in C# or CoreLib; confirm by grepping producers and by a handwritten-IL probe. `Unmanaged` stashing is *not* a new hazard: the legacy retry also writes the raw address after yielding, with the same unchecked trust (`F10.RawMemoryAccessValid`), so plan 13's "stability" objection does not distinguish the two paths. Admitting it is cheap; admitting `Transient` is not (would need tracing a stack-resident value). |
| `newobj` of an intrinsic constructor (`execute_intrinsic_call`) | **Remove** | Completion (`complete_constructor_receiver`) can be the activation's completion action, as `Jump` is for inline calls. Intrinsic constructors **do** suspend: `String::.ctor(char[])`, `.ctor(char, int)`, `.ctor(ReadOnlySpan<char>)` and `.ctor(char*)` yield at safepoints (`constructors.rs`); today `newobj` returns their `Yield` without completing the receiver and re-allocates on retry. Converting the string constructors in the bulk slice depends on this slice. |
| `.cctor` dispatch, span equality slow path, `Expression.Compile` redirect | **Keep** | They dispatch managed bodies, never intrinsics; `Other` is correct for them. |
| `ExecutionEngine::unified_dispatch`/`dispatch_method` public APIs (`ResolvedMethodDispatchMode::ExecutionEngineApi`) | **Remove** (maintainer decision 2026-11-11) | Plan 13 kept them as a host entry contract, but no production code calls them: the only callers are `condition_wait_tests.rs` and `native_substring_tests.rs`. `ExecutionEngineApi` sends intrinsics straight to `intrinsic_call`, i.e. the sync handler, so every async-only intrinsic is unreachable through them. Delete both APIs, `dispatch_resolved_method_for_engine_api`, `unified_dispatch_for_engine_api` and the `ExecutionEngineApi` mode; port those tests to drive guest IL or the inline path. |
| Legacy static-init (`newobj`, `prepare_method_initialization`, `RunClassConstructor`) | **Remove last** | Highest risk: three triggers with different operand contracts, `retrying_initializer`, `initializer_claim` on frames, legacy condvar `wait_for_init`, and the late wrapping that differs from the field path. Do it only after dispatch is inline everywhere, so one `ensure_initialized` effect can serve all callers. |
| Substring valid-small fast path | **Keep** | Measured in plan 13. |
| `monitor_retry` frame field | **Remove if** the Unmanaged/Transient decision leaves no timed byref fallback; otherwise it stays for exactly that. |

## Scope

**In scope:** the "Remove" rows above, and deleting every type, field, trait method and
registration they leave unused.

**Out of scope:** converting synchronous leaf intrinsics; transport for `Transient` pointers
unless the probe shows a producer; the cross-thread GC-handle, finalizer and shared-memory
items in the README.

**Scope-creep guard.** Struct transport is where this plan can grow the way the byref-homes
work did. Admit plain value types (including ones containing object references, which `stash`
already traces) and enums. Do not build `Transient`/`Unmanaged` pointer transport to admit ref
structs; record them in "Kept" instead. If slice 1 needs more than a value-type output shape,
its marshalling and its tests, stop and revise.

## Design

Reuse plan 13's API (body-only registration, `vm.exception(..)` for throws, optional
`fallback`). One new piece of vocabulary is needed up front:

- **Yield-for-safepoint effect.** An async body that loops over chunks needs a cheap
  "offer a safepoint" await point. `fx.vm().await` already waits for a stop, so the likely answer
  is that it is the point; the blocking slice confirms with `GC.Collect` and a bulk loop before committing
  the rest.

## Slices

The original ordering put blocking intrinsics first. A body-only `GC.Collect` fails through
the still-live non-inline reflection and multicast continuations: both
`reflection_retry_completed_collection_does_not_retry` and
`multicast_fallback_completes_collection_targets` return `NotImplemented` in ST and
`multithreading`. Those tests force the fallback through a test seam; before slice 1 the
continuations were also needed for plain value types. So the order is: transport, then the
continuations that transport retires, then the remaining non-inline producers, then the
intrinsics that lose their fallbacks.

1. **Value types across native calls (complete).** Add a value-type shape to `NativeOutput` and the
   argument check, map enums to their underlying type in reflection as multicast already does,
   and admit `Static` byref origins in the multicast argument check. Both selectors stop
   declining these shapes. Continuations are not deleted yet.
   - Tests: multicast and reflection diff fixtures with struct arguments and returns (a struct
     with an object field, `Nullable<int>`, `DateTime`, an enum), each through a target that
     suspends and with a GC between targets; `tracing` asserts `multicast_path = "async"` and
     `reflection_invoke_path = "async"` for them.
   - The selectors retain fallback for value types containing managed pointers. The multicast
     ref-struct fixture checks both argument and return exclusions; the existing static-byref
     fixtures now assert the async path.
   - Validation: VM library tests and the selection and CLR differential fixtures passed in ST
     and `multithreading` with `DOTNET_SKIP_BUILD=1` (new fixtures built on demand).
2. **Multicast continuation.** Delete `MulticastState`, `handle_multicast_step`, the checks at
   `call_ops_impl.rs:1090` and `context.rs:464`, `MULTICAST_FALLBACK_TEST` and the non-inline
   (`synthetic`) branch of `try_start_multicast`, or record in "Kept" exactly which shape keeps
   them (ref-struct arguments, byref return, `TypedReference`).
   - Tests: diff fixtures for a throwing target, ordering, last return value, targets that
     suspend (`Thread.Sleep`, contended `Monitor.Enter`), and targets that trigger static
     initialization. Port `multicast_fallback_completes_collection_targets` to the native path.
3. **Reflection continuation.** Move receiver and argument validation and byref write-back into
   the body. Delete `InvokeReturn`, `FrameReturnAction`, `reflection_pending` with its
   per-instruction check, and the two reflection fallbacks, or record which shape keeps each.
   If any fallback remains, its calls to suspending intrinsics must use a continuation that can
   resume a native activation.
   - Tests: the reflection differential fixtures plus one per previously declined shape; the
     ported collection-target regression; byref-write-back-after-throw fixed or restated; a
     benchmark run showing the per-instruction check's removal.
4. **Intrinsic-constructor `newobj` and `Other` dispatch.** Route guest calls to inline dispatch
   where the completion contract allows it, including the yielding string constructors. Delete
   `NativeDispatchMode::Other` handling that no longer has a producer. Delete the public
   `ExecutionEngine` dispatch APIs and the `ExecutionEngineApi` mode per the audit row.
5. **Blocking intrinsics.** `Thread.Join` (both), `LowLevelLock.Acquire`, the `TryEnter_FastPath`
   family, `GC.Collect` x3. Delete their sync handlers, Join's `monitor_retry_deadline` use and
   any now-unused `MonitorHost` methods. Also settle the Transient/Unmanaged question here and
   either admit `Unmanaged` (deleting the byref monitor fallbacks and, if nothing else uses it,
   `monitor_retry`) or record both shapes in "Kept".
   - Tests: existing join/monitor/GC fixtures in ST and `multithreading`; a timed `Join` whose
     deadline survives a GC stop; `GC.Collect` observed by a finalizer fixture; a unit test per
     removed fallback that fails if the registration reappears.
6. **Bulk intrinsics.** `mem_helpers`/`buffer` first (correctness), then string, then span
   equality, one family per commit, each gated on a benchmark comparison against the parent
   commit. `cpblk`/`initblk` keep progress across the yield even if they stay instructions.
   - Tests: first, a `multithreading` repro for an overlapping 2 MiB+ `Buffer.Memmove` while
     another thread forces collections (expected to fail before the change). Then a large-input
     fixture per family that crosses a GC safepoint and is collected mid-copy, and the benchmark
     numbers recorded in this plan.
7. **Static-init triggers.** One `ensure_initialized` path for `newobj`,
   `prepare_method_initialization`, `RunClassConstructor` **and the native-call preflight in
   `dispatch_pending_native_call`**; delete `retrying_initializer`, frame `initializer_claim`,
   legacy `wait_for_init`, the `original_ip`/`original_stack_height` save-restore, and the late
   wrapping in `unwind_frame`, and update the plan 08 static-initialization slice document.
   - Tests: `statics/cctor_filter_42.cs` and its `newobj`/call/`RunClassConstructor`/delegate/
     reflection variants against the CLR; recursive and cross-thread initialization.
8. **Cleanup.** `grep` every item in the Problem table; remove leftovers; update the README
   "Fallback continuations" and "Legacy static-initialization triggers" entries and plan 08's
   residual-callers text.

## Done when

- Each "Remove" row has no remaining user (`grep` evidence in the slice commit).
- No benchmark in the "Bulk" families regressed beyond what the maintainer accepted (numbers
  recorded here).
- `cargo test` passes in ST and `multithreading`; Miri value/VM jobs and the differential
  harness are green.
- The README items above are rewritten to describe only what remains.

## Kept (fill in as slices land)

| Item | Reason | Test that fails if the reason stops being true |
|---|---|---|
| | | |

## Ask the maintainer only if

- The Unmanaged/Transient probe finds a real `Transient` producer.
- Struct transport needs anything beyond a value-type shape (see the scope-creep guard).
- The overlapping-memmove repro confirms corruption: fix it standalone (as the next plan ID) if
  slice 6 is not close.
- A bulk family regresses more than a few percent on its benchmark.
- Multicast needs a CLR deviation (for example exception behaviour inside an invocation list).
- Reflection ambiguity handling (`AmbiguousMatchException` versus first-best-match) must change
  to delete a fallback.
