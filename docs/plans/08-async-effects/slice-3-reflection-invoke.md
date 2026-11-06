# Plan 08 slice 3: reflection Invoke through transparent calls

**Status:** Notes. Fill in Done-when and open questions before starting.

## Consumer

`dotnet-intrinsics-reflection/src/methods.rs` (`MethodInfo::Invoke`, `ConstructorInfo::Invoke`
overloads): sets `FrameReturnAction::InvokeReturn`, which `stack/context.rs` applies on return.

## Decided

- Transparent calls. Exceptions propagate exactly as they do today, with no
  `TargetInvocationException` wrapping (maintainer decision). Revisit only if a real
  compatibility failure is observed.

## Introduced here

- Managed-pointer descriptors for byref parameters: heap owner plus offset (box payload,
  `object[]` slot) and frame-home serial plus offset. Each is enabled together with its tests:
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

- Is static-field byref transport needed by any fixture? (It is blocked on p12.)
- Constructor result rooting for value-type constructors.

## Done when (draft)

- Reflection fixtures pass. `FrameReturnAction::InvokeReturn` is deleted, or reduced if other
  users remain.
- Byref write-through visible after normal and exceptional return.
