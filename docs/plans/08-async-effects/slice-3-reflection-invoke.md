# Plan 08 slice 3: reflection Invoke through transparent calls

**Status:** Notes. Fill in Done-when and open questions before starting.

## Consumer

`dotnet-intrinsics-reflection/src/methods.rs` (`MethodInfo::Invoke`, `ConstructorInfo::Invoke`
overloads): sets `FrameReturnAction::InvokeReturn`, which `stack/context.rs` applies on return.

## Decided

- Transparent calls. Exceptions propagate exactly as they do today, with no
  `TargetInvocationException` wrapping (maintainer decision). Revisit only if a real
  compatibility failure is observed.

## Verified facts and hazards (HEAD 069c8294, read from code, not reproduced)

- **Three** producers of `FrameReturnAction::InvokeReturn`: `methods.rs:400`
  (`MethodInfo.Invoke`), `methods.rs:436` (`ConstructorInfo.Invoke`) and
  `types/type_construction.rs:240` (`Activator.CreateInstance(Type, object[])`). All three are
  in scope. The action is consumed in `return_frame` from the caller (`context.rs:419-448`)
  and read in `should_honor_tail_call` (`call_ops_impl.rs:1030`).
- **The action lingers.** When the target completes synchronously (an intrinsic, P/Invoke or
  delegate path), nothing consumes it, and it fires at the caller's next frame return. When
  the callee throws and the caller catches, nothing clears it. A `.cctor` triggered by
  `reflection_dispatch_method` consumes it at the `.cctor`'s own return. `substring_fallback_42`
  passes only because pop-and-push is a no-op for `String`. Add regressions for a value-type
  result from an intrinsic target and for throw-then-catch.
- Byrefs today use heap owners only (`methods.rs:859-1079`): value-type `ref` points into the
  existing box (a null argument gets a zeroed box written back to the `object[]` slot);
  reference-type `ref` points at the `object[]` element. No frame-home or static origin
  appears, and no fixture passes a static-field byref. So slice 3 needs heap descriptors only.
  Frame-home descriptors are added by whichever slice first has a consumer (`ReliableEnter`,
  slice 5).
- Constructors push the new instance below the arguments on the caller's stack
  (`methods.rs:378-380,424`); `ConstructorInfo.Invoke` passes `HeapStorage::Obj` even for
  value types. No fixture covers `ConstructorInfo`, `Activator` or `GetConstructor`; add them,
  including a value-type constructor.
- Existing fixtures: `reflection/reflection_invoke_42`, `invoke_byref_42`,
  `invoke_byref_writethrough_42`, `reflection_getmethod_generic_arity_42`,
  `reflection_method_via_attr_42`, `strings/substring_fallback_42`.

## Introduced here

- Managed-pointer descriptors for byref parameters: heap owner plus offset (box payload,
  `object[]` slot). Frame-home serial plus offset only if a slice 3 consumer needs it;
  otherwise slice 5 adds it. Each is enabled together with its tests:
  conversion from `PointerOrigin`, tracing, reconstruction, invalidation on frame retirement.
- Synthetic-frame completion action, if reflection dispatch goes through a frame.

## Carried from the archive

- Byrefs are materialized immediately before pushing the managed frame, inside the same
  mutation. No branded `ManagedPtr` is stored in the future or mailbox.
- Value-type refs address a box (keep the existing box identity; null gets a zeroed box).
  Reference-type refs address the original `object[]` slot (write-through). No deferred
  copy-out that changes aliasing or hides writes on exception.
- Eligibility includes the declared return layout. Do not discover after a side effect that a
  byref return cannot be transported.

## Open questions

- Constructor result rooting for value-type constructors.

(Answered: no fixture needs static-field byref transport, so p12 does not block this slice.)

## Done when (draft)

- Reflection fixtures pass. `FrameReturnAction::InvokeReturn` is deleted, or reduced if other
  users remain.
- Byref write-through visible after normal and exceptional return.
