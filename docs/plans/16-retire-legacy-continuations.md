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

| Legacy mechanism | Users at the start of this plan |
|---|---|
| Sync yield-retry intrinsics (re-run the instruction after `StepResult::Yield`) | `Thread.Join()`/`Join(int)` (`thread.rs`, `join_managed_thread` in `context_ops.rs`), `LowLevelLock.Acquire`, `Monitor.TryEnter_FastPath` family (`monitor.rs`), `GC.Collect` x3 (`gc.rs`, `increment_ip` + `Yield`) |
| Bulk loops that yield at `check_gc_safe_point` | string `operations`/`constructors`/`search`, `mem_helpers`, `buffer`, span `equality`, `cpblk`/`initblk` (`instructions/memory.rs`) |
| `MulticastState` on `StackFrame` (+ `handle_multicast_step`, `dispatch/mod.rs`; `call_ops_impl.rs:1090`, `context.rs:464`) | multicast `Invoke` shapes that `try_start_multicast` (`native_effects.rs`) declines; a native multicast body (`multicast_body`) already serves the rest |
| `FrameReturnAction::InvokeReturn` + `apply_frame_return_actions` + `reflection_pending` | reflection `Invoke`/`CreateInstance` fallbacks for shapes `invoke_select`/`activator_select` decline; `run_reflection_call` already serves the rest |
| Reflection retry test seam | `reflection_retry_completed_collection_does_not_retry` drives a `GC.Collect` target through the legacy continuation |
| `StackFrame::monitor_retry` + `monitor_retry_deadline`/`monitor_clear_retry` | timed byref `Monitor` fallbacks (Transient/Unmanaged `bool&`) and `Thread.Join(int)` (Join removed in slice 5) |
| `NativeDispatchMode::Other` + `fallback =` registration | the residual callers in plan 13 |
| `prepare_method_initialization`, `newobj` and `RunClassConstructor` initialization | legacy static-init triggers (README "Legacy static-initialization triggers") |

### Declined shapes (why the continuations are still live)

Slice 1 added native-call transport for plain value types and enums, and admitted `Static` byref
origins. The remaining declined shapes are:

- Byref arguments require `Heap`, `FrameHome`, `Static`, `Unmanaged` or MT cross-arena origins, and a target
  type without managed pointers.
- Byref returns and `TypedReference` are declined in both.
- Ref-struct arguments (`Span<T>`, `SpanAction`) carry `ManagedPtr` fields whose origins may be
  `Transient` or `Unmanaged`; ref-struct values are excluded by type, regardless of origin.

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
| `Thread.Join` yield loop | **Removed (slice 5)** | A native body waits on worker completion with an absolute deadline. `monitor_retry` is no longer used by Join. |
| `LowLevelLock.Acquire`, `TryEnter_FastPath` | **Removed (slice 5)** | Native bodies use the monitor effect; legacy `monitor_enter_safe` remains for the retained byref fallbacks. |
| `GC.Collect` x3 | **Removed (slice 5)** | Native bodies request a full collection and resume after the executor services it. |
| Bulk string/mem/span/buffer loops | **Remove, measured** | Plan 04 measured a large-call cost for native activations. Each family needs a before/after on its benchmark (`substring`, `end_to_end`, `runtime_primitives`); small-input paths keep an inline fast path as Substring does. A fast path registered as a sync `fallback` is also what any remaining `Other` dispatch receives, so it must either handle every input size or `Other` must have no producer for that intrinsic. Overlapping memmove (above) is a correctness reason, not only a cleanup. |
| `cpblk`/`initblk` instruction yields | **Fix, then decide** | These are instructions, not intrinsics, but share `chunked_copy_with_safe_point` and its restart-from-zero overlap hazard. At minimum keep progress across the yield; whether they become effects is decided with the `mem_helpers` family. |
| Multicast `Invoke` (`MulticastState`) | **Keep for excluded shapes** | Ref-structs, byref returns, `TypedReference` and unsupported byref origins remain excluded; see "Kept". The `retrying_initializer` flag goes with the static-init slice. |
| Reflection `InvokeReturn` / `reflection_pending` | **Removed (slice 3)** | Every remaining declined shape was byref-like, which the CLR refuses with `NotSupportedException` before the target runs; the body now raises that, and Activator's binding and default-value cases. Byref write-back after a throw and ambiguous-match "first best match" remain README deviations. |
| Monitor byref `Transient` / `Unmanaged` fallback | **Removed (slice 5b)** | Slice 5 kept both, but its own probe shows the only `Transient bool&` candidate (`ldflda` on a struct value on the evaluation stack) is invalid IL: ECMA-335 §III.4.11 allows only an object, managed pointer or unmanaged pointer as `ldflda`'s operand (unlike `ldfld`, not a value-type instance), and the CLR rejects it with `InvalidProgramException`. That leaves `Unmanaged`, which is cheap to admit as a byref origin (no new hazard; see slice 5b). |
| `newobj` of an intrinsic constructor (formerly `execute_intrinsic_call`) | **Removed (slice 4)** | Completion (`complete_constructor_receiver`) can be the activation's completion action, as `Jump` is for inline calls. Intrinsic constructors **do** suspend: `String::.ctor(char[])`, `.ctor(char, int)`, `.ctor(ReadOnlySpan<char>)` and `.ctor(char*)` yield at safepoints (`constructors.rs`). `newobj` now dispatches intrinsic constructors in `NativeDispatchMode::Constructor`, so an async constructor body can suspend and the value-type receiver is completed only on success. The synchronous string constructors still return their safepoint `Yield` and re-allocate on retry until slice 6 converts them; that slice should add a managed-exception case showing the receiver is not published. |
| `.cctor` dispatch, span equality slow path, `Expression.Compile` redirect | **Keep** | They dispatch managed bodies, never intrinsics; `Other` is correct for them. |
| `ExecutionEngine::unified_dispatch`/`dispatch_method` public APIs (`ResolvedMethodDispatchMode::ExecutionEngineApi`) | **Removed (slice 4)** | Plan 13 kept them as a host entry contract, but no production code calls them: the only callers are `condition_wait_tests.rs` and `native_substring_tests.rs`. `ExecutionEngineApi` sends intrinsics straight to `intrinsic_call`, i.e. the sync handler, so every async-only intrinsic is unreachable through them. Delete both APIs, `dispatch_resolved_method_for_engine_api`, `unified_dispatch_for_engine_api` and the `ExecutionEngineApi` mode; port those tests to drive guest IL or the inline path. |
| Legacy static-init (`newobj`, `prepare_method_initialization`, `RunClassConstructor`) | **Remove last** | Highest risk: three triggers with different operand contracts, `retrying_initializer`, `initializer_claim` on frames, legacy condvar `wait_for_init`, and the late wrapping that differs from the field path. Do it only after dispatch is inline everywhere, so one `ensure_initialized` effect can serve all callers. |
| Substring valid-small fast path | **Keep** | Measured in plan 13. |
| `monitor_retry` frame field | **Removed (slice 5b)** | Timed Monitor byrefs now use native effects. |

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

The original ordering put blocking intrinsics first. A body-only `GC.Collect` would fail through
the still-live non-inline reflection continuation: `reflection_retry_completed_collection_does_not_retry`
constructs that route. Before slice 1, multicast also needed the legacy continuation for plain
values; slice 2 moved its collection regression to the native path and recorded its remaining
exclusions. Slice 3 then removed the reflection continuation; the remaining order is non-inline producers, then blocking intrinsics.

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
2. **Multicast continuation (complete with retained exclusions).** Delete `MulticastState`, `handle_multicast_step`, the checks at
   `call_ops_impl.rs:1090` and `context.rs:464`, `MULTICAST_FALLBACK_TEST` and the non-inline
   (`synthetic`) branch of `try_start_multicast`, or record in "Kept" exactly which shape keeps
   them (ref-struct arguments, byref return, `TypedReference`).
   - Tests: diff fixtures for a throwing target, ordering, last return value, targets that
     suspend (`Thread.Sleep`, contended `Monitor.Enter`), and targets that trigger static
     initialization. Port `multicast_fallback_completes_collection_targets` to the native path.
   - The forced-fallback hook is removed. The collection fixture now asserts three async
     selections and compares all three calls with the CLR. The legacy continuation and synthetic
     branch remain for the cases in "Kept"; they are revisited in slices 3 and 4.
   - Validation: VM library tests, the selection test and all multicast CLR differential tests
     passed in ST and `multithreading` with `DOTNET_SKIP_BUILD=1`. A source search confirms the
     forced multicast fallback hook and its test have no remaining code users.
3. **Reflection continuation (complete).** Move receiver and argument validation and byref write-back into
   the body. Delete `InvokeReturn`, `FrameReturnAction`, `reflection_pending` with its
   per-instruction check, and the two reflection fallbacks, or record which shape keeps each.
   If any fallback remains, its calls to suspending intrinsics must use a continuation that can
   resume a native activation.
   - Tests: the reflection differential fixtures plus one per previously declined shape; the
     ported collection-target regression; byref-write-back-after-throw fixed or restated; a
     benchmark run showing the per-instruction check's removal.
   - Progress: `MethodInfo`/`ConstructorInfo` target and argument validation now runs in the
     native body for transportable signatures. Retained reflection dispatch and retry call
     targets inline, including async-only intrinsics; a retry must not enqueue a native target
     again when its activation suspends. The collection fixture asserts all three calls select
     the native path. Exceptional byref write-back is still the documented CLR deviation: the
     body passes pointers into the original box or argument array, so writes are immediate even
     on a throw. Changing that requires a separate copy-in/copy-out design and tests for aliasing.
   - Excluded shapes: every value the native transport cannot root is byref-like (a `ref struct`,
     `Span<T>`, `TypedReference`). A CLR probe showed it refuses all of them before running the
     target: `NotSupportedException("Cannot create boxed ByRef-like values.")` for a parameter
     (by value or by reference) and for an `Activator` target, and
     `NotSupportedException("Specified method is not supported.")` for a return. The fallback
     instead ran these targets, returned boxed ref structs, or failed with an internal error
     (`TypedReference`). The body now raises the CLR exceptions; byref-like detection reads
     `IsByRefLikeAttribute` through `type_attributes`, because the loader parses attributes
     lazily and `TypeDefinition::attributes` stays empty.
   - Activator: the body resolves the target and raises `ArgumentNullException`,
     `ArgumentException` for an open generic, `NotSupportedException` for a byref-like type, and
     `MissingMethodException` ("Constructor on type 'X' not found." or the abstract-class
     message) as the CLR does. With no arguments, a value type without a matching constructor
     (including primitives, which previously threw `Arg_MustBeType`) returns its boxed default
     through a new no-call preparation result (`NativeReflectionPlan::Value`). Arrays no longer
     construct a `DotnetRs.Array`, and `List<>` no longer constructs an instance. `string` keeps
     `ArgumentException("Arg_MustBeType")`: constructing one needs the intrinsic-constructor
     protocol, recorded in the README.
   - Deleted: both selectors' `Fallback` outcomes, the `Invoke` arms of
     `runtime_method_info_intrinsic_call`, `intrinsic_activator_create_instance_type_args`,
     `FrameReturnAction`/`InvokeReturn`, `StackFrame::frame_continuation` and
     `reflection_pending`, `ReflectionPending`, `CallState::reflection_arming`,
     `NativeActivation::reflection_actions`, `apply_frame_return_actions`,
     `retry_reflection_target`, `reflection_dispatch_method`, `set_frame_return_action`, the
     per-instruction pending check in `step_normal`, and the two unit tests that constructed the
     continuation by hand. `StackFrame` shrinks from 600 to 568 bytes. No guest route reaches the
     `Invoke`/`Activator` intrinsics non-inline: retained `MulticastState` targets carry an
     excluded shape, which these signatures lack, and the engine API goes in slice 4.
   - Tests: `invoke_byref_like_42.cs` (27 native selections, no target runs) matches the CLR
     line for line; every reflection selection fixture asserts no fallback.
   - Benchmark: four alternating Criterion pairs per configuration (`bench-fat`, CPU 0 on an
     AMD Ryzen 9 9900X, separate targets and Criterion homes, `--sample-size 10 --warm-up-time 1
     --measurement-time 4`), baseline `0c5c58f8`, comparing mean point estimates:

     | Workload | Pair 1 | Pair 2 | Pair 3 | Pair 4 | Mean |
     |---|---:|---:|---:|---:|---:|
     | ST dispatch | -1.27% | -0.63% | -0.50% | -0.69% | -0.77% |
     | ST string | -1.91% | -1.78% | +8.18% | -1.13% | +0.84% |
     | ST reflection | +0.64% | -1.09% | +0.20% | +0.68% | +0.11% |
     | MT dispatch | -3.38% | -1.68% | -2.08% | -1.81% | -2.24% |
     | MT string | -9.00% | -7.56% | -9.16% | -8.04% | -8.44% |
     | MT reflection | -4.36% | +0.67% | -0.53% | +0.92% | -0.83% |

     Whole-process `perf stat --test` (two pairs each) measured 0.53–0.60% fewer user
     instructions and 0.79–0.80% fewer branches in both configurations. The fixture DLLs were
     built per target directory from identical sources and are not byte-identical, unlike plan
     08's audit. `reflection` (`ReflectionEnumeration_0`) enumerates members and does not call
     `Invoke`; it is a control. The MT string gain is consistent but not attributed.
   - Inlining hazard: without the check, LLVM inlined `step_normal` into `ExecutionEngine::run`
     (189 KB). Under `multithreading` the merged loop stored each `StepResult` with a narrow
     store and reloaded it with 16-byte loads, so `dispatch` ran 41% slower (+42.4, +41.6,
     +39.4, +41.8%), `string` 33% and `reflection` 17%, with 1.5% fewer instructions, 34% more
     cycles and 60% more branch misses; `perf annotate` put 54% of `run`'s samples on the
     reload. `step_normal` is now `#[inline(never)]`, which restores the baseline's
     out-of-line structure (`run` is again `0xdfb` bytes in MT).
4. **Intrinsic-constructor `newobj` and `Other` dispatch (complete).** Route guest calls to inline dispatch
   where the completion contract allows it, including the yielding string constructors. Delete
   `NativeDispatchMode::Other` handling that no longer has a producer. Delete the public
   `ExecutionEngine` dispatch APIs and the `ExecutionEngineApi` mode per the audit row.
   - Intrinsic `newobj` admits native activations with the correct operand and result conventions:
     reference constructors return their object, while value-type receiver homes are completed
     only after success. Constructor activation starts after its completion is installed.
     Existing synchronous string constructors keep their safepoint retry until slice 6 converts
     the bulk loops.
   - Deleted the engine APIs, synthetic multicast activation and its return machinery. `Other`
     remains for the excluded multicast continuation and managed-only dispatches.
   - `native_newobj_completes_string_and_value_receiver_after_gc` verifies suspended activation
     completion across collection and checks that failures do not publish the receiver; existing
     string, span, multicast and threading fixtures cover synchronous constructors and retained
     dispatch. The contended multicast fixture is now
     ignored without `multithreading`, as its CLR comparison already was.
   - Validation: workspace nextest without features (926 passed) and MT (1,204 passed with the
     documented reflection race excluded); the reflection race passed separately after its CLR
     sanity check failed under parallel load. A timer comparison also failed under parallel load
     and passed both individually and in the subsequent run. VM library tests, multicast
     selection and workspace clippy passed in ST and MT; 20 multicast CLR comparisons and
     string differential tests passed. `check.sh --fast` was stopped during its silent fixture
     build after several minutes; the workspace checks used available fixtures instead.
5. **Blocking intrinsics (complete).** `Thread.Join` (both), `LowLevelLock.Acquire`, the `TryEnter_FastPath`
   family, `GC.Collect` x3. Delete their sync handlers, Join's `monitor_retry_deadline` use and
   any now-unused `MonitorHost` methods. Also settle the Transient/Unmanaged question here and
   either admit `Unmanaged` (deleting the byref monitor fallbacks and, if nothing else uses it,
   `monitor_retry`) or record both shapes in "Kept".
   - The retained `MulticastState` (see "Kept") calls each target through non-inline dispatch,
     where an async intrinsic without a fallback returns `NotImplemented`. Today only targets
     whose own signature has an excluded shape reach it, and the one such async intrinsic,
     `Monitor.Enter(object, ref bool)` with an `Unmanaged` flag, keeps its fallback. Admitting
     `Unmanaged` in `stash` and deleting the Monitor byref fallbacks therefore also requires
     admitting `Unmanaged` origins in the multicast byref check (`try_start_multicast`);
     otherwise a multicast `ref bool` delegate over an unmanaged flag breaks.
   - Tests: existing join/monitor/GC fixtures in ST and `multithreading`; a timed `Join` whose
     deadline survives a GC stop; `GC.Collect` observed by a finalizer fixture; a unit test per
     removed fallback that fails if the registration reappears; a registry test that every
     async intrinsic without a fallback has a signature the native multicast and reflection
     selectors admit, so no retained continuation can reach it.
   - Both Join overloads wait on worker completion notifications; timed joins keep the absolute
     deadline across GC stops. `GC.Collect` requests a full collection and yields until it is
     serviced; contention for the collection coordinator leaves the request pending.
   - The byref Monitor fallbacks remain for both `Transient` and `Unmanaged`, along with the
     retry field and MonitorHost methods they use. Producer search found `ldflda` on a stack
     value as a candidate; the handwritten IL probe is rejected by the CLR with
     `InvalidProgramException`. No other guest `Transient bool&` producer was established.
   - `join_timeout_survives_collection`, `completion_wait_wakes_on_publication_and_after_publication`,
     `collection_request_retries_when_coordinator_is_busy`, the async registry guards and the CLR
     IL probe cover the new paths and retained exclusions.
   - Validation: workspace nextest passed in ST (929 tests) and `multithreading` (1,211 tests,
     with the reflection race filter used in slice 4); VM clippy passed in both configurations.
     The finalizer and multicast collection fixtures and the CLR IL probe passed separately.
     Source search finds no `threading_join_managed_thread`, `intrinsic_thread_join`,
     `intrinsic_gc_collect`, or `intrinsic_monitor_try_enter_fast_path`; `monitor_enter_safe`
     remains only for the retained byref Monitor fallback.
   - Review fix: `Start` marks a thread started before it publishes the worker completion, so a
     `Join` in that window captured an absent completion and waited forever. The completion
     handle now resolves the completion again until it is published
     (`managed_join_resolves_completion_published_after_start`, which fails without the fix).
   - Review notes for slice 8: `GC.Collect` was the only synchronous target that advanced the
     caller's IP and then yielded, so the `completed_yield` handling in
     `dispatch_pending_native_call` and the IP comparison in `handle_multicast_step` look dead;
     confirm and remove. `VmEffects::collect` resumes after one yield even when another thread
     holds the collection coordinator; the request stays pending, so fix the doc comment (or
     loop) rather than claim the collection has run.
5b. **Retire the byref Monitor fallbacks (complete).** Admit `Unmanaged` as a byref origin and treat
   `Transient` as an invalid program, then delete the fallbacks slice 5 kept.
   - Admit `Unmanaged` as a byref **argument origin** in native transport (`stash` and its
     reconstruction) and in the multicast byref check (`try_start_multicast`), under the same
     target-type rule as the other origins (no managed pointers in the target type). The
     address is not traced; rooting is a no-op and reconstruction yields the same address. This
     is the same unchecked trust the legacy retry already places in the raw address
     (`F10.RawMemoryAccessValid`); write that justification at the admission site. Do not admit
     `Unmanaged` or `Transient` pointers stored in ref-struct fields: ref structs stay excluded
     by type, so the scope-creep guard above still holds.
   - `Transient` byref operands decline in the selectors before any operand is consumed and
     raise `System.InvalidProgramException` ("Common Language Runtime detected an invalid
     program."), matching the CLR's verdict on the only known producer. If a managed exception
     cannot be raised before commitment, raise it from the body before any side effect and say
     so in the slice notes. Before deleting anything, recheck every `PointerOrigin::Transient`
     construction site for a producer reachable from valid IL; if one exists, stop and ask.
   - Delete `fallback =` from the five byref registrations (`ReliableEnter`, `Enter(object,
     bool&)`, `TryEnter(object, int, bool&)`, `ReliableEnterTimeout`,
     `TryEnter_FastPath_WithTimeout`), the sync handlers `intrinsic_monitor_reliable_enter` and
     `intrinsic_monitor_try_enter_timeout_ref`, `monitor_enter_safe` (and the timeout variant if
     unused), `monitor_retry_deadline`, `monitor_clear_retry`, `MonitorRetry`,
     `StackFrame::monitor_retry` and all its resets (`executor.rs`, `dispatch/mod.rs` x3,
     `exception_ops_impl.rs` x2, `context.rs`, `stack.rs`), and any `MonitorHost` method left
     unused. Update the `StackFrame` layout guard (`layout_sizes.rs`) and its comment.
   - Tests: port `monitor_legacy_unmanaged_timeout_retains_deadline_across_stop` to the native
     path (an `Unmanaged` flag, a timed enter that waits across a real collection, and the flag
     written exactly once on success and left false on timeout); a native multicast `ref bool`
     delegate over an unmanaged flag in ST and MT; a `Transient` selection test showing no
     operand is consumed and `InvalidProgramException` is raised; flip the unmanaged-byref case
     in `multicast_selection_precedes_operand_commitment` from excluded to admitted, keeping its
     CLR differential fixture green; change `blocking_intrinsics_have_no_sync_fallback` so the
     byref overloads must have no fallback. Existing monitor, `lock` and threading fixtures in
     ST and `multithreading`.
   - Benchmark: not required; no hot path changes. Confirm `StackFrame` shrinks.
   - Result: `Unmanaged` byref arguments preserve their raw address across polls; raw integer and unmanaged-pointer operands are canonicalized using the declared target type. `Transient`
     selectors throw before operand commitment. The only production constructors are the
     object-instruction and Unsafe memory helpers: only `ldflda` can return the transient address,
     and a value-type stack operand there is invalid IL. The five sync fallbacks, retry record,
     resets and unused MonitorHost methods are deleted. `StackFrame` shrank from 568 to 560 bytes.
     `rg 'monitor_retry|MonitorRetry|intrinsic_monitor_reliable_enter|intrinsic_monitor_try_enter_timeout_ref|monitor_enter_safe' crates`
     finds no remaining production users. Validation passed the VM library suites in ST and MT,
     both layout guards, and the multicast, threading and lock CLR fixtures in both configurations.
     The unmanaged timed-wait test covers successful acquisition and timeout across a collection,
     including exactly one flag write. The multicast fixture covers a `ref bool` Monitor call.
6. **Bulk intrinsics (partial: memory family complete; string and span-equality families open).** `mem_helpers`/`buffer` first (correctness), then string, then span
   equality, one family per commit, each gated on a benchmark comparison against the parent
   commit. `cpblk`/`initblk` keep progress across the yield even if they stay instructions.
   - Tests: first, a `multithreading` repro for an overlapping 2 MiB+ `Buffer.Memmove` while
     another thread forces collections (expected to fail before the change). Then a large-input
     fixture per family that crosses a GC safepoint and is collected mid-copy, and the benchmark
     numbers recorded in this plan.
   - Memory family (done): the repro failed before the change (`ObjectRef codec host invariant
     violation: OwnerUnavailable`, exit 101): a synchronous handler that yields is retried from
     its start with its operands restored, so a partial overlapping move was re-run over
     already-moved bytes and the retained owner was gone. `Buffer.Memmove` (4 registrations),
     `SpanHelpers.ClearWithoutReferences` (2) and the byte-compatible `SpanHelpers.Fill<T>` now
     have async bodies (`memmove_async`, `clear_async`, `fill_async`) that root the pointers as
     descriptors and call `bulk_copy_chunk`/`bulk_fill_chunk` per 1 MiB chunk, re-resolving and
     range-checking both operands each time. Copy direction is a pure function of the unchanged
     addresses and `done` counts in iteration order, so progress survives every yield. Selectors
     pick the async body only above one chunk; smaller operations keep a single-step sync
     handler that cannot reach a safepoint (a measured-fast-path fallback, as for Substring).
   - `cpblk`, `initblk`, `Unsafe.CopyBlock*` and `Unsafe.InitBlock*` no longer poll mid-copy:
     they run to completion. This deviates from "keep progress across the yield": it removes the
     restart hazard instead, at the cost of a collection request waiting for a long block
     (bounded by memcpy time, at most 4 GiB for the `uint` forms).
   - Kept: `SpanHelpers.Fill<T>` for non-byte `T` (and null or non-pointer destinations) still
     uses the element loop with a per-element safepoint poll, which restarts from the beginning.
     It is idempotent but shares the stale-operand hazard; convert it with the string families.
   - Tests: `memmove_overlap_gc_42` and `bulk_fill_gc_42` (both against the CLR, `multithreading`
     only, each crossing GC stops from a forcing thread). Workspace nextest passed in ST
     (809 in vm, cli, unsafe) and `multithreading` (1,210; the known timer fixture flaked once and
     passed on rerun); clippy clean in both.
   - Benchmark (review, `bench-fat`, `taskset -c 0`, four alternating pairs against
     `1bb0f642`, mean change): ST `unsafe_buffer` +0.01%, `memory` +0.15%, `span` +0.46%,
     `json` +1.56%, `string` −1.07%; MT `unsafe_buffer` +0.39%, `memory` +0.38%, `span` +0.30%,
     `json` −0.36%, `string` +7.18% (+5.6 to +9.0%). The MT `string` change is code placement,
     not work: `perf stat --test` instructions and branches match to 0.01%, and the hottest
     function (`intrinsic_string_is_null_or_white_space`, 42–45% of steady-state cycles) is
     byte-identical but moved so its hot loop now straddles a 64-byte line. Accepted.
   - `cpblk`/`initblk` running to completion is accepted: the CLR's block-copy helpers do not
     poll for GC either, and the delay is bounded by one memcpy.
   - Found, not fixed: `Span<byte>.Clear()` on a 3 MiB span takes ~42 s and never reaches
     `clear_select`, on the parent commit too; its registered signature is not the one `Clear`
     calls, so `clear_async` has no fixture. Needs its own investigation.
   - Open: the string family (constructors, concat, copy/insert/replace/search loops at
     `constructors.rs`, `operations.rs`, `search.rs`) and span/string equality
     (`equality.rs`, `operations.rs`) still return `Yield` from sync handlers and, with the
     constructors, still re-allocate on retry; each needs its own commit and benchmark.
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

## Kept

| Item | Reason | Test that fails if the reason stops being true |
|---|---|---|
| `MulticastState`, its step/return/tail-call checks, and initializer retry | Ref-struct values or byref targets containing managed pointers, byref returns, `TypedReference`, and unsupported or mismatched shapes cannot use native transport. | `multicast_selection_precedes_operand_commitment` checks the exclusions and admits unmanaged byrefs; CLR differential fixtures check execution. Transient byrefs are rejected as invalid. |

## Ask the maintainer only if

- The Unmanaged/Transient probe finds a real `Transient` producer.
- Struct transport needs anything beyond a value-type shape (see the scope-creep guard).
- The overlapping-memmove repro confirms corruption: fix it standalone (as the next plan ID) if
  slice 6 is not close.
- A bulk family regresses more than a few percent on its benchmark.
- Multicast needs a CLR deviation (for example exception behaviour inside an invocation list).
- Reflection ambiguity handling (`AmbiguousMatchException` versus first-best-match) must change
  to delete a fallback.
