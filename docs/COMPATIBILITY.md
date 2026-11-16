# Compatibility Notes

`dotnet-rs` runs framework-dependent applications by reading their
`runtimeconfig.json` and `deps.json` files. The explicit `--assemblies` path
remains available for direct assembly-root selection.

The host path has been validated at three levels:

- the repository's managed fixture suite;
- a Newtonsoft.Json application that serializes and deserializes an object,
  with matching output and exit status under stock `dotnet` and `dotnet-rs`;
- an EF Core InMemory smoke application that creates, saves, and queries an
  entity with matching output and exit status under both runtimes.

The EF Core work exercised generic method context propagation, reflection and
nullability metadata, by-reference value-type marshalling, and method identity
during query translation. Focused regression fixtures for those runtime fixes
live under `crates/dotnet-cli/tests/fixtures/reflection/`.

## Validated runtime surface

The enabled CLR differential fixtures also validate these runtime behaviors:

- Static `FieldInfo.GetValue`/`SetValue` use initialized static storage for primitive,
  value, reference and native-pointer fields, including generic and thread statics and
  `initonly` checks (`reflection_static_fields_match_dotnet`,
  `pointer_static_fields_match_dotnet`, and `reflection_thread_static_match_dotnet`).
- Registered handlers cover bodiless methods in the loaded `System.Math` and
  `System.MathF` definitions. `math_extern_42` checks deterministic rounding and
  special-value behavior; its `Stopwatch` check is a sanity comparison rather than
  an assertion on elapsed time.
- Both `Array.Fill<T>` overloads and overlapping `Span<T>.CopyTo` paths are exercised.
  Reference-free span elements use `DotnetRs.Internal.Memmove`; reference-containing
  elements use direction-aware typed stores.
- Signed managed byref displacements are exercised below their origin, including
  subtract-and-add-back behavior over arrays, fields and locals. Callee-owned generic
  arguments are exercised through constrained dispatch and native-effect resumption.
- `stdout_before_internal_error_matches_dotnet` compares the guest-output fixture with
  `dotnet`. The debug-only `stdout_is_preserved_on_internal_error_exit_with_pipe_and_file_capture`
  regression verifies the bytes after executor shutdown through both pipe and file
  capture; it does not compare the injected VM failure exit against the CLR's successful exit.

The implementation paths are in
[`dotnet-intrinsics-core/src/math.rs`](../crates/dotnet-intrinsics-core/src/math.rs),
[`dotnet-assemblies/src/support/Array.cs`](../crates/dotnet-assemblies/src/support/Array.cs),
[`dotnet-assemblies/src/support/Span.cs`](../crates/dotnet-assemblies/src/support/Span.cs),
[`dotnet-value/src/pointer/mod.rs`](../crates/dotnet-value/src/pointer/mod.rs),
[`dotnet-types/src/members.rs`](../crates/dotnet-types/src/members.rs), and
[`dotnet-intrinsics-reflection/src/fields.rs`](../crates/dotnet-intrinsics-reflection/src/fields.rs).

This is compatibility evidence, not a claim of general EF Core or .NET runtime
compatibility. EF Core SQLite, broad framework coverage, `AssemblyBuilder`/`TypeBuilder`,
networking, and complete asynchronous scheduling remain outside the validated
surface.

## Dynamic code and Reflection.Emit

The VM reports `RuntimeFeature.IsDynamicCodeSupported` true and `IsDynamicCodeCompiled` false
(`crates/dotnet-vm/src/intrinsics/app_context.rs`), because EF Core treats a false
`IsDynamicCodeSupported` as NativeAOT. Libraries therefore take their interpreted or reflection
paths:

- **Expression trees** compile through the Linq interpreter
  (`LambdaExpression.CanCompileToIL` and `DelegateHelpers.CanEmitObjectArrayDelegate` report false,
  as under NativeAOT). Delegates for up to 16 parameters, none byref, pointer or byref-like, are
  built by a support-assembly thunk table. Byref or pointer delegate types and more than 16
  parameters throw `PlatformNotSupportedException`.
- **System.Text.Json** reflection serialization uses its reflection member accessor through a VM
  seam (`DefaultJsonTypeInfoResolver.MemberAccessor`) instead of its Emit accessor.
- **`System.Reflection.Emit.DynamicMethod`** is replaced by a recording stub in the support
  assembly. This is a spike, on a branch only
  ([plan 24](plans/24-reflection-emit-investigation.md), landing recommendation pending the
  maintainer's decision). Where it is present, it runs `Invoke` and `CreateDelegate` on methods
  that use straight-line code, calls to non-generic methods, `newobj`, `throw` and typed
  `try`/`catch`/`finally`. It rejects with `NotSupportedException`: labels and branches, field
  operands, `calli`, filters and faults, generic or nested types and callees, byref signatures,
  opcodes outside a table of about 70, and `DynamicILInfo`/`GetTokenFor`. `owner`, module and
  `skipVisibility` do not grant private access, compiled methods are never freed, and stack-trace
  frames were not compared with `dotnet`.
- **`AssemblyBuilder`, `TypeBuilder` and `ModuleBuilder`** are not supported and not planned;
  no application rung needs them.

[Plan 25](plans/25-dynamicmethod-implementation.md) covers extending `DynamicMethod` to the
operations real libraries use.

## Remaining boundaries

Managed byref displacements use a signed 32-bit compact range, so positive
displacements above `i32::MAX` are not representable. A negative displacement may
be formed and carried, but an access still has to resolve within its live owner's
range; out-of-range dereferences are rejected. See
[`ManagedPtr::checked_offset`](../crates/dotnet-value/src/pointer/mod.rs) and
[`ManagedPtr::checked_byte_offset`](../crates/dotnet-value/src/pointer/mod.rs).

Reference-containing span copies continue to use typed stores rather than a raw
bulk copy. Resumable bulk operations and general GC write-barrier coverage for bulk
memory operations are not established by these fixtures. The Math coverage walk
checks handler registration for loaded bodiless methods; the differential fixture
does not assert arbitrary transcendental last-bit results across platforms.
