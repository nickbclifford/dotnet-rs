# Plan 08 slice 2: multicast delegates through transparent calls

**Status:** Complete
**Outcome:** Admitted multicast invocations use transparent native calls in both configurations,
preserving target order, return values and exception behavior. The existing `MulticastState`
continuation remains for shapes the effects transport cannot safely represent.

## Consumer and current path

The bodyless delegate dispatch recognizes `Invoke` outside the attribute-generated intrinsic
registry. `VesContext::try_start_multicast` selects admissible calls before consuming their
operands; `multicast_body` invokes their targets through `call`. Unsupported shapes use
`invoke_delegate` and `MulticastState`, whose dummy frame is intercepted by
`ExecutionEngine::handle_multicast_step`. Single-target dispatch remains synchronous. The
[delegate dispatch documentation](../../DELEGATES_AND_DISPATCH.md#multicast-invocation) describes
the two paths.

`CallState` holds a stack of activations with owned operands, outputs and root sets. Managed
children return through generation-addressed destinations on their frames, and `call` also
handles synchronous and suspending intrinsic targets. Finalizers run in a separate managed
segment. The facade roots scalar and object values and heap and frame-home byref descriptors;
the multicast completion validator checks the declared transportable result shape. See the
[effects contract](../08-async-effects.md) for operand ownership, rooting and completion rules.

## Design

Use the activation stack, monotonic `NativeId`, generation-addressed single-slot mailbox and
transparent `call` effect described in the [main plan](../08-async-effects.md). Assign each
effect's reply generation on its first poll. Put the native
return destination on the **managed child `StackFrame`**, next to `frame_continuation`; it
identifies the waiting activation and effect generation. A frame travels with plan 07's suspended
segments, unlike an active-stack index, and the destination is reset on frame reuse. This avoids
a separate side table and its frame-identity and retirement bookkeeping. `HandlerAddress` remains
an index into the active stack and is not native linkage.

The delegate dispatch path must enter the native body directly: bodyless `Invoke` is classified
by `try_delegate_dispatch`, not by an annotated intrinsic registration. Retain the main plan's
synthetic-frame return completion where dispatch needs it; do not assume the slice-1 inline-only
admission or string-only result validator covers delegate invocation. The body consumes its
operands before its first call, retains the invocation list and arguments under the rooting
contract, calls targets in order and publishes the last return (or no return for void). Synchronous
targets and managed targets both complete the same effect, with each reply addressed to its
request generation. The effect handler fetches and copies values across activation root sets
before dropping the source root; it never forwards a `DynamicRoot`.

Select the path before consuming the receiver or arguments or performing any target side effect.
Admit scalar and object shapes and supported heap and frame-home byrefs only when every argument
and declared return shape can be transported safely. Add the descriptors needed by those
consumers, with the conversion, tracing, reconstruction and invalidation rules in the main plan.
Keep static-field byrefs, other excluded pointer categories, byref-containing aggregates, declared
byref returns and any other unsupported shape on the existing multicast continuation. If admission
cannot establish a shape is supported, choose the fallback; never attempt fallback after commitment.
Preserve single-target dispatch. Do not copy byrefs in and out: targets must see the original
storage and aliases. Fallback selection does not authorize creation of an unmanaged origin for a
managed pointer.

This is a scoped exception to deleting the old multicast machinery, not a second new scheduler or
frame-stack protocol. Retain `MulticastState`, its frame trace/reset, the `step_normal` intercept,
`handle_multicast_step`, and the return and tail-call guards only as needed by the fallback. They
must not be used by admitted multicast calls. Removal of the fallback requires a later plan decision
covering *all* remaining excluded shapes; fixing p12 alone is not sufficient.

`dispatch_method_with_mode` can start a type initializer and return before dispatching the
requested target. The call effect must retain its pending target and arguments until dispatch
actually completes or the target frame is installed. A `.cctor` return is not the target's reply:
retry the pending dispatch using the existing initialization/auto-return behavior, without
advancing the multicast body or delivering the initializer's return. If a synchronous intrinsic
has already dispatched a different target, retain that target and its staged arguments rather
than replaying the intrinsic; preserve its caller-side return action across the initializer and
apply the action before replying. This is part of handling the outstanding call, not a separate
frame-stack or scheduling protocol.

The retained continuation has the same initializer hazard. Fix it there too: a target whose dispatch
starts a `.cctor` remains pending until the target is actually dispatched, and the initializer return
is not treated as a target return. Retain arguments and target identity across the retry; do not
advance the invocation list twice or repeat target side effects. A throwing initializer follows the
existing managed exception path.

The driver must distinguish a runnable activation from one parked on a managed child: replace
the unconditional native polling at `ExecutionEngine::step` and `run`, and move activations with
their suspended segments. Poll neither an activation waiting on a child nor one in a suspended
segment. Bound inline effect completions per engine entry at 32, independently of the access-grant
budget. Generalized terminal cleanup retires children, disables the bridge and drops futures
before their roots, without running managed code in `Drop`.

Exception search treats the native boundary as logical: transparent search continues outward
without resuming Rust, including before the frame-zero unhandled fallback. An outside handler
causes child finally/fault blocks to run and the future to be dropped when unwind crosses the
boundary; an inside handler keeps the parent alive. Stack dumps show one synthetic frame per
boundary with the intrinsic name, native ID and IL offset, not Rust poll frames.

Isolate finalizers in a separate managed segment with a separate ambient exception context; they
must not push frames onto a parked caller or into an interrupted filter or handler. Finalizers may
run while a native operation is parked, as authorized in the main plan. Native calls inside filters
and handlers use plan 07's nested segments. A managed child's tail call or `jmp` must retain its
return-to-native destination and boundary; either preserve the destination on replacement or decline
tail-call optimization. `jmp` must carry the destination.
Transparent `call` suffices; `try_call` is not needed here.

## Compatibility hazards

- A type initializer started during a target dispatch returns as auto-invoked. Without retry
  bookkeeping, `handle_multicast_step` mistakes the target arguments for a prior return and skips
  the target. The initializer fixtures check the ordering and final result on both paths.
- Multicast arguments are not limited to Substring's scalar and object operands. In particular,
  a multicast `ref` argument into a caller local is supported today and both targets can update
  it. Preserve supported argument and return shapes, including write-through, while applying the
  main plan's managed-pointer and out-of-scope rules. Never retain a branded pointer in the future
  or mailbox, and do not use an unmanaged origin for a managed pointer. Exercise the actual
  origins needed by the consumer rather than assuming all byrefs are heap pointers. Route
  out-of-scope shapes to the retained continuation before committing operands.
- Finalizer isolation must protect an interrupted filter or handler as well as parked native
  work. Frame-home lookup and serial issuance span the finalizer and parked segments.
- `dispatch_method_jmp` carries `constructor_result` and `native_return` through frame
  replacement. Frame pooling clears per-call destinations on reset and retirement.

## Done when

- In ST and MT, a real-frame test body exercises transparent calls, nested activations, reply
  generations, operand commitment, the inline-completion bound and terminal cleanup. Child
  returns are copied between root sets; wrong-set or stale replies are internal errors.
- Focused exception tests cover an outer handler, an inner handler, child finally/fault ordering
  and an unhandled boundary; managed-only exception and filter behavior stays unchanged.
- Tests cover finalizers during parked work and in filters/handlers, including isolation of a
  finalizer exception; native calls within filters/handlers; activation movement across nested
  suspended segments; and tail calls and `jmp` in managed children.
- Admitted multicast uses the async body in both configurations, with a test that proves path
  selection rather than inferring it from output. All existing delegate fixtures pass;
  CLR-diffed fixtures cover a throwing target with outer filters, a target catching internally,
  nested multicast, multicast in a filter and a `.cctor` in a target. Heap and frame-home
  multicast byref write-through and aliasing regressions exercise the async path in both modes.
- The static-field byref aliasing fixture passes against CLR in both modes and proves fallback
  selection before commitment. Additional tests cover an unsupported declared return or aggregate,
  untouched operands on fallback, and calls nested across the two paths. CLR-diffed fallback
  fixtures exercise a `.cctor` in a target and exception/filter ordering. Verify that targets run
  once, in order, and that finalizer and suspended-filter handling remain correct on both paths.
- The retained multicast state and its dispatch, return, tracing, reset and tail-call handling are
  used only for fallback invocations. The single-target path remains synchronous.

## Multicast regressions

`multicast_selection_precedes_operand_commitment` observes the actual selection events for
scalar, heap-byref and frame-home-byref calls and for static-field and aggregate fallbacks.
The CLR-diffed `delegate_multicast_refs_42`, `delegate_multicast_ref_cctor_42`,
`delegate_multicast_cctor_42`, `delegate_multicast_static_ref_cctor_42` and
`delegate_multicast_static_ref_42` cover write-through, aliasing and initializer order.
`delegate_multicast_filters_42` and `delegate_multicast_static_ref_filters_42` exercise exceptions,
inner handlers and nested filters on both paths; `delegate_multicast_finalizers_42` collects and
waits for finalizers inside both paths. `delegate_multicast_nested_42` covers both nesting
directions and a wrapped single target, while `delegate_multicast_reflection_42` covers
synthetic return completion and nested intrinsic dispatch across an initializer. The scalar,
aggregate and many-target fixtures cover return admission and bounded inline completions.

`rooted_frame_home_descriptor_preserves_aliases_and_rejects_retired_home` and
`rooted_heap_pointer_descriptor_keeps_owner_alive` check descriptor reconstruction and lifetime.
`native_foreign_byref_owner_survives_collection_and_child_call` and
`native_cross_arena_byref_owner_survives_collection_and_child_call` use the production cross-arena
reference codec and coordinated collection while the byref descriptor is the only root.
