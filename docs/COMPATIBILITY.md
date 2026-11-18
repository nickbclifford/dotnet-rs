# Compatibility Notes

`dotnet-rs` runs framework-dependent applications by reading their
`runtimeconfig.json` and `deps.json` files. The explicit `--assemblies` path
remains available for direct assembly-root selection.

The host path has been validated at three levels:

- the repository's managed fixture suite;
- a Newtonsoft.Json application that serializes and deserializes an object,
  with matching output and exit status under stock `dotnet` and `dotnet-rs`
  (`crates/dotnet-cli/tests/host_projects/newtonsoft_roundtrip`);
- an EF Core InMemory smoke application that creates, saves, and queries an
  entity with matching output and exit status under both runtimes
  (`crates/dotnet-benchmarks/fixtures/ef_inmemory`).

A dependency-injection host project (`crates/dotnet-cli/tests/host_projects/dependency_injection`)
is also run as a host rung for dynamic-code changes; its matching output does not prove that the
provider's background compilation succeeded (see [plan 25](plans/25-dynamicmethod-implementation.md)).
Run the host projects with `scripts/diff_run.sh --host` and `DOTNET_RS_BIN` set to a release CLI.

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
- Under `multithreading`, GC handles are VM-wide, finalizers run on a dedicated finalizer
  thread, and canonical strings and reflection objects keep one identity across threads; see
  [GC and Memory Safety](GC_AND_MEMORY_SAFETY.md#shared-gc-handle-roots).

The implementation paths are in
[`dotnet-intrinsics-core/src/math.rs`](../crates/dotnet-intrinsics-core/src/math.rs),
[`dotnet-assemblies/src/support/Array.cs`](../crates/dotnet-assemblies/src/support/Array.cs),
[`dotnet-assemblies/src/support/Span.cs`](../crates/dotnet-assemblies/src/support/Span.cs),
[`dotnet-value/src/pointer/mod.rs`](../crates/dotnet-value/src/pointer/mod.rs),
[`dotnet-types/src/members.rs`](../crates/dotnet-types/src/members.rs), and
[`dotnet-intrinsics-reflection/src/fields.rs`](../crates/dotnet-intrinsics-reflection/src/fields.rs).

This is compatibility evidence, not a claim of general EF Core or .NET runtime
compatibility. EF Core SQLite, broad framework coverage and networking remain outside the
validated surface. Known gaps are tracked under
[Not yet planned](plans/README.md#not-yet-planned) in the plans README.

## Dynamic code and Reflection.Emit

The VM reports `RuntimeFeature.IsDynamicCodeSupported` true and `IsDynamicCodeCompiled` false.
EF Core needs the former. `IsDynamicCodeCompiled` stays false because Newtonsoft's Emit path would
then need `DynamicMethod.InitLocals`, which the recording `DynamicMethod` does not implement.

- **Expression trees** use the Linq interpreter. Generic support-assembly thunks handle up to 16
  parameters without byref, pointer or byref-like types; other shapes use the framework's emitted
  object-array thunk. `CanCompileToIL` stays false because EF InMemory hits an unimplemented
  reflection method when it is true.
- **System.Text.Json** selects its Reflection.Emit member accessor from
  `IsDynamicCodeSupported`, so it runs over `DynamicMethod`
  (`crates/dotnet-cli/tests/fixtures/reflection/json_serializer_poco_42.cs`).
- **`System.Reflection.Emit.DynamicMethod`** uses a recording support-assembly implementation.
  It supports branches, fields, indirect calls, exception regions, generic and nested types,
  byref signatures and modules. Stack traces through dynamic methods are compared with `dotnet`
  (`dynamic_method_stack_trace_matches_dotnet`). Raw `DynamicILInfo` is unsupported, and compiled
  synthetic assemblies are not yet collected (see
  [Not yet planned](plans/README.md#not-yet-planned)). See
  [plan 25](plans/25-dynamicmethod-implementation.md).
- **`AssemblyBuilder`, `TypeBuilder` and `ModuleBuilder`** are not supported and not planned.

## Remaining boundaries

Without `multithreading`, a finalizer runs in a separate managed segment with its own stable
`Thread` identity and logical monitor/initializer owner key; arena allocation identity and pointer
serialization are unchanged. Automatic finalizers are deferred while the interrupted thread owns a
monitor or has an initializer claim, then retried at later safepoints. A long-held monitor can
therefore delay automatic finalization until it is released. `GC.WaitForPendingFinalizers` forces
processing; if a finalizer needs a monitor or type initializer held by the waiting caller, the VM
reports a single-threaded deadlock rather than suspending and resuming the caller through a
scheduler.

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
