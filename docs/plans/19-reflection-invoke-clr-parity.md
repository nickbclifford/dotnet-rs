# Reflection invocation matches the CLR

**Status:** Not started
**Outcome:** Every reflection-invoke fixture, including those now excluded from CLR diffing
(`invoke_byref_exception`, `invoke_nullable`, `invoke_byref`, `invoke_byref_writethrough`,
`activator_create_instance_args`, `get_constructor_types`, `reflection_invoke`), matches `dotnet`
through `scripts/diff_run.sh` and the diff harness.

## Problem

[Plan 08](08-async-effects.md) made reflection calls transparent and rejected declared byref
returns. Both were maintainer decisions taken while reflection still needed a caller-frame
continuation. [Plan 16](16-retire-legacy-continuations.md) slice 3 deleted that continuation:
`MethodInfo.Invoke`, both `ConstructorInfo.Invoke` overloads and
`Activator.CreateInstance(Type, object[])` now run as native effect bodies
(`invoke_async`, `crates/dotnet-intrinsics-reflection/src/methods.rs`; `activator_async`,
`types/type_construction.rs`), both through `run_reflection_call`. The maintainer reversed both
decisions on 2026-11-13 in favor of matching the CLR.

Current deviations:

- **No `TargetInvocationException`.** `run_reflection_call` uses `fx.call`, so a guest throw
  crosses the boundary unchanged. The `invokeAttr` operand is ignored, so
  `BindingFlags.DoNotWrapExceptions` has no meaning. `Activator.CreateInstance<T>()` and
  `RuntimeType.CreateInstanceDefaultCtor` (`type_construction.rs`) push constructor frames
  directly and are transparent too.
- **Byref returns rejected.** `invoke_select` returns host `NotImplemented` (exit 255) for a
  declared byref return (`invoke_byref_return_255.cs`,
  `reflection_byref_return_is_rejected_before_target_runs`).
- **Argument binding** (`crates/dotnet-intrinsics-reflection/src/invoke_args.rs`):
  - `reference_fits` accepts any vector, and `heap_concrete` returns `None` for vectors, so
    array arguments and receivers are not type-checked.
  - `plan_argument` rejects every byref argument whose box is not exactly the parameter type;
    the CLR widens and writes the converted value back.
  - The `Nullable<T>` branch accepts only an exact boxed `T`, no primitive widening
    (`invoke_nullable_42.cs` check 13 asserts the non-CLR `ArgumentException`).
  - Byref writes go through pointers into the caller's box or `object[]` slot, so they are
    visible after the target throws (`invoke_byref_exception_42.cs` asserts this).
- **Overload selection.** `select_constructor` and `handle_get_constructor_impl`
  (`types/type_members.rs`) take the first best match instead of throwing
  `AmbiguousMatchException`.
- **Strings.** `activator_target` groups `System.String` with `Void` and pointers and throws
  `ArgumentException("Arg_MustBeType")`. The reflection constructor path allocates via
  `reflection_new_object_with_lookup`, which cannot build a string, so
  `typeof(string).GetConstructor(..).Invoke(..)` is likely broken as well (untested).

## Scope

In: the four effect-body entry points above, the two direct-frame Activator paths,
`GetConstructor(Type[])`, and the fixtures and docs that encode the deviations.

Out:
- `Binder`, `CallingConventions` and `ParameterModifier` arguments to `GetConstructor`: they are
  popped and ignored today, and no fixture or known application needs a custom binder.
- `Delegate.DynamicInvoke`, `Type.InvokeMember`, `Reflection.Emit`: not implemented paths;
  write separate entries if needed.
- The multicast continuation's byref-return exclusion
  ([DELEGATES_AND_DISPATCH.md](../DELEGATES_AND_DISPATCH.md)): delegates, not reflection.

## Design

- **Wrapping.** Mirror `VmEffects::ensure_initialized`, which already observes a callee exception:
  call `fx.try_call` unless `invokeAttr` has `DoNotWrapExceptions` (0x02000000), and on
  `NativeFailure::Managed(inner)` return a `TargetInvocationException` built by a new backend
  method modeled on `wrap_initializer_failure`, using `throw_by_name_with_inner`. Host errors stay
  host errors. Capture stops exception search at the boundary, so outer filters see only the
  wrapper, as on the CLR. The direct-frame Activator paths get the same treatment by routing
  them through the effect body rather than adding a second wrapping mechanism.
- **Byref returns.** Drop the selector rejection. `validate_invoke_return` rejects a byref-like
  pointee with `NotSupportedException`; `reflection_result` reads the returned `ManagedPtr` with
  the pointee type and boxes it. The returned pointer is already transportable (non-transient
  origins).
- **Write-back.** Replace write-through with the CLR's copy-in/copy-out where the CLR does it.
  Slice 3 starts with a CLR probe to pin the exact rule (in-place mutation of an exact value-type
  box vs. copy-back of reference slots and converted boxes; behavior after a throw). This
  supersedes slice 3 of plan 08's "deferred copy-in/copy-out is not equivalent" requirement.
- **Assignability.** Array checks reuse the `castclass`/`isinst` assignability code (array
  covariance, enum/underlying element compatibility, generic interfaces) rather than a
  reflection-local rule.

## Slices

1. **`TargetInvocationException` wrapping.** Wrapping and `DoNotWrapExceptions` in
   `run_reflection_call`; route `Activator.CreateInstance<T>()` and `CreateInstanceDefaultCtor`
   through the same body. A synchronous intrinsic target that throws (`reflection_tail.fs`) must
   reach the capture. Update `invoke_byref_exception_42.cs` to catch the wrapper (its write-back
   assertions stay until slice 3), and add a fixture covering wrapping, `InnerException` identity,
   `DoNotWrapExceptions`, a filter outside `Invoke`, and the generic and default-constructor
   Activator paths; diff it against the CLR. Update `reflection_path_selection` counts.
2. **Byref returns.** Replace `invoke_byref_return_255.cs` and its integration test with a `_42`
   fixture: value and reference pointees, a ref to a static, an array element and a field, a
   byref-like pointee (`NotSupportedException`), and a null ref return (probe the CLR first).
   Diff against the CLR.
3. **Argument binding.** CLR probe for write-back, then: array type checks for arguments and
   receivers, byref widening with write-back, `Nullable<T>` widening, and no write-back after a
   throw if the probe confirms. Rewrite `invoke_nullable_42.cs` check 13 and the write-back
   assertions in `invoke_byref_exception_42.cs`; add both, `invoke_byref_42.cs` and
   `invoke_byref_writethrough_42.cs` to the diff harness. Confirm `invoke_argument_race_42`
   still races a narrowing (`long` → `int`) conversion.
4. **Overload ambiguity and strings.** `AmbiguousMatchException` from `GetConstructor(Type[])` and
   `Activator.CreateInstance(Type, object[])`; string construction through Activator and
   `ConstructorInfo.Invoke` using the existing intrinsic-constructor path. Extend
   `activator_create_instance_args_42.cs` and `get_constructor_types_42.cs` with ambiguous and
   string cases and add both, plus `reflection_invoke_42.cs`, to the diff harness.
5. **Docs.** Remove the two README "Not yet planned" reflection entries. Mark the reversed
   decisions in plan 08 (out-of-scope list, transparent-call text, reflection fallback paragraph),
   plan 08 slice 3, plan 10, plan 13 (stale fallback text) and plan 16 (byref returns,
   write-back, `Arg_MustBeType`, ambiguity) as superseded by this plan. Update
   `EXCEPTION_HANDLING.md` (reflection is a second `try_call` consumer) and the code comments in
   `type_construction.rs` and `methods.rs`.

## Done when

- Every fixture named in the outcome is registered in the diff harness and matches `dotnet` in
  ST and MT, with and without `validation-all`.
- No reflection fixture encodes a deviation, and no fixture is kept out of diffing for policy
  reasons.
- The README has no reflection-invoke entries, and plan 08's decision list points here.

## Ask the maintainer only if

- The CLR probe shows write-back behavior that cannot be reproduced without giving up the
  aliasing guarantees plan 08 slice 3 tested (box identity across collection, foreign owners).
- Wrapping requires a second capture mechanism beyond `try_call`.
- String construction through reflection needs a new constructor protocol rather than reusing
  `dispatch_intrinsic_constructor`.
