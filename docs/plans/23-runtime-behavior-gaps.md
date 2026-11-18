# Runtime behavior gaps: static field reflection, Math FCalls, Array.Fill, Span.CopyTo, signed byref offsets and generic arity

**Status:** Complete
**Outcome:** The enabled differential fixtures for static `FieldInfo.GetValue`/`SetValue`, bodiless
Math/MathF methods, `Array.Fill`, overlapping `Span<T>.CopyTo`, signed byref offsets and generic
context match `dotnet` in both base feature sets. The buffered-stdout report is closed with its
pipe/file repro evidence.

## Resolved behavior

- **Static field reflection** uses the declaring type's initialized static storage (the same path
  and type-initializer check as `ldsfld`/`stsfld`) and throws `FieldAccessException` for
  `initonly` statics after initialization, as the CLR does from .NET Core 3.0 on.
- **Math and MathF** handlers cover every bodiless method of the loaded CoreLib's `System.Math` and
  `System.MathF`; `math_and_mathf_bodiless_methods_have_intrinsic_handlers`
  (`crates/dotnet-vm/src/intrinsics/metadata.rs`) walks the loaded definitions.
- **`Array.Fill<T>`**: both overloads are in the support `Array`.
- **`Span<T>.CopyTo`** handles overlap: reference-free elements use `DotnetRs.Internal.Memmove`;
  reference-containing elements use direction-aware typed (barriered) stores.
- **Signed byref offsets.** Managed pointers carry signed 32-bit displacements, so a byref may be
  formed below its origin; access is still checked against the owner's extent (see
  [Managed pointer displacement and access](../GC_AND_MEMORY_SAFETY.md#managed-pointer-displacement-and-access)).
- **Generic arity.** Intrinsics read callee generic slots only through
  `MethodDescription::own_method_arg`/`own_type_arg` (`crates/dotnet-types/src/members.rs`), which
  check the declaring arity; constrained calls and resumed native effects keep the target context.
- **Buffered stdout.** Guest bytes survive the internal-error exit path without a flush change
  (`stdout_is_preserved_on_internal_error_exit_with_pipe_and_file_capture`).

The validated surface is summarized in [COMPATIBILITY.md](../COMPATIBILITY.md).

## Remaining limits

- Positive byref displacements above `i32::MAX` are not representable (plans README, "Not yet
  planned").
- `Span<T>.CopyTo` of reference-containing `T` stays element-wise; there is no bulk GC barrier
  ("Resumable bulk operations and barriers").
- Reflection access checks beyond `initonly`/literal are not modeled.
- Only `Fill` was added to the support `DotnetRs.Array`; other missing members are found as
  fixtures need them.

## Checks

The differential fixtures named in [COMPATIBILITY.md](../COMPATIBILITY.md#validated-runtime-surface),
in the ten feature configurations (`cargo run -p xtask -- matrix test-features`). The Newtonsoft
and EF host rungs still match.
