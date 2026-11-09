# Plan 08 slice 3: reflection Invoke through transparent calls

**Status:** Complete
**Outcome:** Admitted reflection calls use transparent native calls in ST and MT, return correctly
boxed results and preserve byref write-through. Corrected legacy return handling remains only
for reflection calls excluded from effects.

## Consumers and current path

`runtime_method_info_intrinsic_call` handles `MethodInfo.Invoke` and both
`ConstructorInfo.Invoke` overloads. `intrinsic_activator_create_instance_type_args` handles
`Activator.CreateInstance(Type, object[])`. Admitted calls to each use transparent calls; the
synchronous fallback for excluded shapes and non-inline dispatch sets `InvokeReturn` on the caller.
`VesContext::return_frame` consumes that action to turn void into null, box value results or pass
reference results through. `should_honor_tail_call` prevents replacement of that caller.
`dispatch_pending_native_call` and `finish_synthetic_native_return` also account for the action
when a synchronous intrinsic dispatches another target. The parameterless generic Activator
intrinsic is a separate path, not this consumer.

A synchronous target need not return a managed frame, so fallback return handling must cover
both synchronous completion and managed returns. A later return or a caught exception must not
consume an action belonging to another invocation. Initializers require
particular care: the slice 2 pending-call retry keeps a `.cctor` return separate from the
requested target and preserves an action across that retry. Correct the fallback's action
handling without regressing nested multicast or initializer ordering.

`unmarshal_invoke_params` constructs heap-owner byrefs: a value-type ref points into the
existing boxed payload (a null argument gets a zeroed box written into the `object[]`), and a
reference-type ref points into the original `object[]` slot. Writes are immediate, including
on an exceptional return. These are byrefs to a value or an object reference, not pointers
stored in the array: ECMA-335 §§II.14.4.2 and III.1.1.5.2 permit those targets but forbid a
managed pointer to another managed pointer or a managed pointer stored in an array, field or
static. The observed reflection marshalling does not require static or transient pointer
transport. The slice 2 `RootedValue` descriptor supports heap, MT cross-arena and frame-home
origins; use the actual origin rather than guessing from the address.

## Design

Select before consuming operands, modifying the argument array, allocating a replacement box,
running a constructor or dispatching a target. Check actual transport categories and the
declared return layout, including rejecting unsupported byref returns before any side effect.
Preserve existing behavior for excluded shapes under the main plan's legacy-path rules; do not
turn a managed pointer into an `Unmanaged` pointer. Reuse the existing heap and frame-home
root descriptors and their tracing, reconstruction and retired-home checks. The reflection
consumer needs the heap-backed box and array-slot cases; MT also needs foreign owners covered
through the production codec. Materialize byrefs immediately before the managed frame is pushed,
inside the same mutation. Neither the future nor the effect-control state retains a branded `ManagedPtr`.
Preserve the original box and array-slot identity; deferred copy-in/copy-out is not equivalent.

The selector's box check is a snapshot. In multithreaded builds, another mutator can replace an
`object[]` value box before preparation; `unmarshal_invoke_params` checks the box again and raises
a managed `System.ArgumentException` for a mismatch. `NativePrepareError::Managed` carries that
exception into the activation root set before the VM dispatches it, matching the CLR rather than
turning the race into a host error.

Call the resolved target through `fx.call`, retaining the existing virtual-dispatch and generic
lookup behavior. Consume operands before the first call. The async body handles void, value and
reference returns itself, with boxing performed while VM access is live; exceptions propagate
unchanged, without `TargetInvocationException`. The current facade roots scalars, object
references and supported byref descriptors, not arbitrary `StackValue::ValueType` aggregates:
admission must establish that the declared result can be transported or provide a safe
same-mutation boxing path before such a value crosses the boundary. Do not fail after the target
has executed merely because its result cannot be rooted. Preserve synthetic-frame completion and
the slice 2 pending-target/initializer protocol for managed and synchronous targets.

### Scoped reflection fallback

The existing dispatch contract sends all non-inline modes, including a tail-prefixed intrinsic
call, to the synchronous fallback before selection. Unsupported transport shapes also need a
fallback. Deleting the sole caller-side return action while retaining those rules would lose the
required reflection result transformation. Keep `InvokeReturn` and the frame continuation, or
their corrected equivalents, only for those fallback calls. Do not route an excluded call to
effects, retry it after commitment, or reject a previously supported call to avoid the fallback.
The existing selector's early rejection of unsupported declared byref returns is unchanged.

Correct the existing continuation so it belongs to the particular outstanding invocation, not
the next arbitrary frame return. It must apply exactly once to that target's successful result,
including a synchronous intrinsic or P/Invoke, and be cleared on a throw, dispatch or host error,
abort and frame retirement. A caught exception must leave no action for a later call. If target
dispatch starts a type initializer, keep the requested target and its arguments pending, retry
the target after initialization, and never treat the initializer as the target's completion or
repeat an already executed target. Nested reflection, multicast, native boundaries, filters and
finalizer segments must not overwrite or consume another invocation's action. Trace any added
GC-bearing state and preserve the caller and its tail-call guard until the pending work ends.
Extending the existing continuation with invocation-specific state and initializer retry is
authorized; it must not introduce an independent frame stack or scheduler. Use the existing
native pending-target protocol where applicable. Keep exception propagation transparent and
use managed errors for supported guest failures rather than panics.

### Constructor result rooting

For a newly constructed reference type, keep the allocated instance as an activation-owned
`RootedValue` across the call and return that reference only after successful completion. For a
value type, allocate a zero-initialized **boxed** instance with the resolved concrete generic
layout, root the box in the activation before any suspension, and pass a heap-owner descriptor
to its payload as `this`. Reconstruct that byref at dispatch, and return the rooted box after
the void constructor completes. This both keeps reference fields traced and lets constructor
writes update the result without retaining a branded value or a transient address. The existing
`call_frame` canonicalizes an object receiver for value-type instance methods, but that does not
itself root the result in a parked native activation; the explicit rooted owner does. For the
overload given an existing receiver, preserve the existing receiver's identity rather than
allocating a replacement.

Apply the runtime's normal boxing rules where the boxed representation requires normalization,
notably `Nullable<T>` (ECMA-335 §I.8.2.4), and keep concrete generic lookup throughout. On a
throw, release the activation and do not publish the constructor result. Do not run managed code
in `Drop`. This design requires only the existing heap descriptor, not a new pointer category or
an ECMA-forbidden container. A temporary simple value-type constructor probe passed on both VM
configurations and CLR; it did not exercise collection, generic layout or nullable boxing.

## Done when

- In ST and MT, admitted `MethodInfo.Invoke`, both `ConstructorInfo.Invoke` overloads and
  `Activator.CreateInstance(Type, object[])` use transparent calls. Path-selection tests show
  admission before commitment and rejection of unsupported origins and declared byref returns
  before side effects; no supported guest failure is converted into a VM panic.
- Heap byrefs to a box payload and to an `object[]` slot retain identity and aliasing across
  collection and write through on normal and exceptional returns. Tests cover actual origin
  conversion, tracing, reconstruction and invalidation for every enabled descriptor category,
  including a production-decoded foreign owner in MT. Existing frame-home retirement tests stay
  enabled. No branded pointer survives in a future or the effect-control state.
- Value results, including a synchronous intrinsic target, are boxed; reference and void results
  have the correct reflection shape. A throw followed by a catch does not leave return handling
  behind. Exceptions remain transparent; CLR comparisons of throwing cases account for the
  deliberately different wrapper policy. Initializer retries, virtual dispatch, nested multicast
  and synthetic completion preserve target ordering and return values.
- ConstructorInfo and Activator fixtures cover reference and value constructors, including a
  value-type instance with a reference field kept alive across an actual collection while its
  constructor is parked. Verify result contents, identity where applicable, generic layout,
  supported nullable boxing and exceptional cleanup in both modes. Existing reflection and
  substring fixtures stay green; applicable new fixtures have real-CLR `diff_test!` entries.
- Prove the corrected continuation is reachable only from the reflection fallback and never
  from an admitted call. Cover fallback success and throw/catch for managed and synchronous
  targets, void/value/reference results, constructor results, initializer success and failure,
  nested invocation, and later unrelated returns. Exercise a real tail-prefixed reflection
  call, verify the emitted IL contains the prefix, and test it in ST and MT; a simulated
  `Other` mode alone is insufficient. Include an unsupported transport shape that remains
  supported on the fallback. Compare applicable results and initializer ordering with CLR,
  accounting for the deliberate difference in exception wrapping.
- Keep only the continuation, trace/reset, tail-call guard and slice 2 forwarding needed by
  that fallback; audit all producers and consumers for the boundary above and update directly
  affected runtime documentation. Keep the pending target and native-return protocols.
- Build fixtures and inspect discovery and results for the affected filters in ST and MT; run
  the full fixture set in both modes as final verification. Run the focused pointer and VM checks,
  affected pinned Miri singletons, doctests where the facade changes, doc drift, and the required
  fast and full checks. Record test counts, Miri limitations and any performance
  evidence separately rather than treating one lane as proof of another.

## Verification (2026-11-09)

- Native: the reflection filter selections passed 39/39 ST and 42/42 MT; the vm-ops and
  reflection-crate tests passed 19/19 ST and 20/20 MT. The MT-only CLR-diffed admission-race
  regression was discovered and passed 1/1 (316 filtered); it is correctly absent in ST because
  the race requires concurrent mutators. Fixture build passed.
- CLR: the admission-race fixture matched the real CLR. Its racer uses a bounded volatile-work
  loop because the attempted `Thread.SpinWait` QCall is unsupported by the VM.
- Native matrix: `NEXTEST_TEST_THREADS=1 ./check.sh --fast` passed 811/811 ST, 1002/1002 MT
  and 1017/1017 MT+validation tests (14/7/9 existing skips).
- Miri: not run for the admission-race change; it changed no unsafe code. Doctests: vm-ops
  passed 6/6 in each configuration. Performance: not measured; no throughput claim.
- Documentation: doc drift passed 56/56; intra-doc-link rustdoc, formatting and diff checks
  passed. Rustdoc reported the existing private-link warning in `managed_ptr_decode_cache.rs`.
