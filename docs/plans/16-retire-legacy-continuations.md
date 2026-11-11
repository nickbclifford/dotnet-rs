# Retire legacy continuations and move suspending intrinsics to effects

**Status:** Not started
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
| `MulticastState` on `StackFrame` (+ `handle_multicast_step`, `dispatch/mod.rs`; `call_ops_impl.rs:1090`, `context.rs:464`) | multicast delegate `Invoke` only (`invoke.rs`) |
| `FrameReturnAction::InvokeReturn` + `apply_frame_return_actions` + `reflection_pending` | reflection `Invoke`/`CreateInstance` fallbacks for shapes `invoke_select`/`activator_select` decline |
| `StackFrame::monitor_retry` + `monitor_retry_deadline`/`monitor_clear_retry` | timed byref `Monitor` fallbacks (Transient/Unmanaged `bool&`) and `Thread.Join(int)` |
| `NativeDispatchMode::Other` + `fallback =` registration | the residual callers in plan 13 |
| `prepare_method_initialization`, `newobj` and `RunClassConstructor` initialization | legacy static-init triggers (README "Legacy static-initialization triggers") |

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
| Bulk string/mem/span/buffer loops | **Remove, measured** | Plan 04 measured a large-call cost for native activations. Each family needs a before/after on its benchmark (`substring`, `end_to_end`, `runtime_primitives`); small-input paths keep an inline fast path as Substring does. |
| `cpblk`/`initblk` instruction yields | **Decide** | These are instructions, not intrinsics. Either run them as effects or keep the chunked loop with `check_gc_safe_point` (it needs no per-call state). Default: keep, record in "Kept". |
| Multicast `Invoke` (`MulticastState`) | **Remove, hardest** | Needs a native body that loops targets with `try_call`, collects the last return value, and propagates exceptions per ECMA-335 §II.14.6 (invocation list order, return of last target). The `retrying_initializer` flag exists only for legacy init and goes with the static-init slice. |
| Reflection `InvokeReturn` / `reflection_pending` | **Partly remove** | Declines are for argument shapes the transport does not admit (wrong receiver type, array args, byref widening). The CLR throws for most of these; moving the validation into the body (plan 13's throw helper) deletes the fallback for the throw cases. Byref write-back after a throw (README) is a CLR deviation to fix in the same body. Ambiguous-match cases need the "first best match" behaviour decided (see below). |
| Monitor byref `Transient` / `Unmanaged` fallback | **Verify, then likely keep** | `lock` statements use a local (`FrameHome`), admitted. A `Transient` `bool&` has no obvious producer in C# or CoreLib; confirm by grepping producers and by a handwritten-IL probe. `Unmanaged` stashing is *not* a new hazard: the legacy retry also writes the raw address after yielding, with the same unchecked trust (`F10.RawMemoryAccessValid`), so plan 13's "stability" objection does not distinguish the two paths. Admitting it is cheap; admitting `Transient` is not (would need tracing a stack-resident value). |
| `newobj` of an intrinsic constructor (`execute_intrinsic_call`) | **Remove** | Completion (`complete_constructor_receiver`) can be the activation's completion action, as `Jump` is for inline calls. Which intrinsic constructors suspend today? Probably none; if so this is just routing `InlineCall`. Check first. |
| `.cctor` dispatch, span equality slow path, `Expression.Compile` redirect | **Keep** | They dispatch managed bodies, never intrinsics; `Other` is correct for them. |
| `ExecutionEngine::unified_dispatch`/`dispatch_method` public APIs | **Keep** | Host entry contract is non-inline (plan 13). |
| Legacy static-init (`newobj`, `prepare_method_initialization`, `RunClassConstructor`) | **Remove last** | Highest risk: three triggers with different operand contracts, `retrying_initializer`, `initializer_claim` on frames, legacy condvar `wait_for_init`, and the late wrapping that differs from the field path. Do it only after dispatch is inline everywhere, so one `ensure_initialized` effect can serve all callers. |
| Substring valid-small fast path | **Keep** | Measured in plan 13. |
| `monitor_retry` frame field | **Remove if** the Unmanaged/Transient decision leaves no timed byref fallback; otherwise it stays for exactly that. |

## Scope

**In scope:** the "Remove" rows above, and deleting every type, field, trait method and
registration they leave unused.

**Out of scope:** converting synchronous leaf intrinsics; transport for `Transient` pointers
unless the probe shows a producer; the cross-thread GC-handle, finalizer and shared-memory
items in the README.

## Design

Reuse plan 13's API (body-only registration, `vm.exception(..)` for throws, optional
`fallback`). One new piece of vocabulary is needed up front:

- **Yield-for-safepoint effect.** An async body that loops over chunks needs a cheap
  "offer a safepoint" await point. `fx.vm().await` already waits for a stop, so the likely answer
  is that it is the point; slice 1 confirms with `GC.Collect` and a bulk loop before committing
  the rest.

## Slices

1. **Blocking intrinsics.** `Thread.Join` (both), `LowLevelLock.Acquire`, the `TryEnter_FastPath`
   family, `GC.Collect` x3. Delete their sync handlers, Join's `monitor_retry_deadline` use and
   any now-unused `MonitorHost` methods. Also settle the Transient/Unmanaged question here and
   either admit `Unmanaged` (deleting the byref monitor fallbacks and, if nothing else uses it,
   `monitor_retry`) or record both shapes in "Kept".
   - Tests: existing join/monitor/GC fixtures in ST and `multithreading`; a timed `Join` whose
     deadline survives a GC stop; `GC.Collect` observed by a finalizer fixture; a unit test per
     removed fallback that fails if the registration reappears.
2. **Bulk intrinsics.** String, `mem_helpers`, `buffer`, span equality, one family per commit,
   each gated on a benchmark comparison against the parent commit.
   - Tests: a large-input fixture per family that crosses a GC safepoint and is collected
     mid-copy; the benchmark numbers recorded in this plan.
3. **Multicast delegates.** Native body replacing `MulticastState`; delete `handle_multicast_step`,
   the frame field, and the checks at `call_ops_impl.rs:1090` and `context.rs:464`.
   - Tests: diff fixtures for multicast with a throwing target, ordering, last-return-value,
     a target that suspends (`Thread.Sleep`, contended `Monitor.Enter`), and a multicast whose
     targets trigger static initialization.
4. **Reflection residue.** Move receiver/argument validation and byref write-back into the body
   and delete `InvokeReturn`, `FrameReturnAction`, `reflection_pending` and the two reflection
   fallbacks, or record exactly which shape forces each to stay.
   - Tests: the reflection differential fixtures plus one per previously declined shape, and the
     byref-write-back-after-throw deviation fixed or restated.
5. **Intrinsic-constructor `newobj` and `Other` dispatch.** Route to inline dispatch where the
   completion contract allows it. Delete `NativeDispatchMode::Other` handling that no longer has a
   producer, or document the remaining producers.
6. **Static-init triggers.** One `ensure_initialized` path for `newobj`,
   `prepare_method_initialization` and `RunClassConstructor`; delete `retrying_initializer`,
   frame `initializer_claim`, legacy `wait_for_init` and the late wrapping in `unwind_frame`, and
   update the slice-4 document.
   - Tests: `statics/cctor_filter_42.cs` and its `newobj`/call/`RunClassConstructor`
     variants against the CLR; recursive and cross-thread initialization.
7. **Cleanup.** `grep` every item in the Problem table; remove leftovers; update the README
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
- A bulk family regresses more than a few percent on its benchmark.
- Multicast needs a CLR deviation (for example exception behaviour inside an invocation list).
- Reflection ambiguity handling (`AmbiguousMatchException` versus first-best-match) must change
  to delete a fallback.
