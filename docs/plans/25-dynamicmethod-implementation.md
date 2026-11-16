# Implement `DynamicMethod`

**Status:** Slice 1 complete; slices 2–5 remain
**Outcome:** `System.Reflection.Emit.DynamicMethod` runs on the VM for every `ILGenerator`
operation a real library uses: branches and labels, fields, `calli`, filters and faults, generic
and nested types and callees, byref signatures, and the owner, module and `skipVisibility`
overloads. System.Text.Json serializes a POCO through its own `ReflectionEmitMemberAccessor`
(no VM seam), the Linq object-array delegate path handles byref and pointer delegate types, and
the differential fixtures for all of it match `dotnet`. The Newtonsoft and EF InMemory host rungs
still match `dotnet` after every slice.

Follows [plan 24](24-reflection-emit-investigation.md), whose spike (a recording `DynamicMethod`
stub and a synthetic one-method module) is the starting point. Closes the README "Not yet planned"
entry for failures that need `DynamicMethod` (byref delegate types, libraries that pick Emit from
`IsDynamicCodeSupported`) once slice 4 lands.

## Problem

The spike runs three fixtures but is deliberately narrow
([plan 24, spike result](24-reflection-emit-investigation.md#spike-result-2026-11-15-all-three-fixtures-pass)):

- **Front end** (`crates/dotnet-assemblies/src/support/DynamicMethod.cs`): the stub's
  `ILGenerator` records `(opcode, immediate, operand)` and rejects labels and every branch opcode,
  field operands, `calli`, filters, faults, signature operands and any opcode outside a table of
  about 70. `maxstack` is fixed at 32. `DynamicMethod.GetParameters`, `MethodHandle`,
  `ReturnTypeCustomAttributes`, `Define*Parameter` and `GetDynamicILInfo` are missing.
- **Back end** (`crates/dotnet-intrinsics-reflection/src/dynamic_method.rs`, intrinsic
  `CompileRecorded`): signatures translate only primitives, `object`, `string`, non-generic
  non-nested classes and value types, and SZ arrays. Generic callees, generic or nested types and
  byref parameters or returns are rejected. Every compile adds a resolution to the metadata arena
  and nothing is evicted.
- **Semantics:** `owner`, module and `skipVisibility` are validated or ignored. The synthetic method
  has a declaring type `DynamicMethods` in an assembly `DynamicMethod$N`, where the CLR reports no
  declaring type. Stack traces were not compared with `dotnet`. Concurrent compiles, GC of delegates
  over dynamic methods and the `NEXT_DYNAMIC_ASSEMBLY` naming were not tested.
- **Per-library seams** added by plan 24 slice 3 exist only because `DynamicMethod` did not work
  (`crates/dotnet-vm/src/intrinsics/linq_expressions.rs`): `CanCompileToIL` and
  `CanEmitObjectArrayDelegate` return false, `DelegateHelpers.CreateObjectArrayDelegate` runs a
  support-library thunk table (at most 16 parameters, no byref/pointer/byref-like), and
  `DefaultJsonTypeInfoResolver.get_MemberAccessor` returns STJ's reflection accessor.
- **Wall found while reaching `DynamicMethod`** (plan 24 slice 2): STJ's
  `ReflectionEmitMemberAccessor.CreateParameterlessConstructor` stops at `Type.Module`, because
  `DotnetRs.RuntimeType.GetModule` is an extern with no intrinsic and `DotnetRs.Module` is empty.

## Scope

In:

- Everything in "Problem" under front end, back end and semantics.
- Whether the slice 3 seams and the feature values change (slice 4).
- A `Module` reflection object sufficient for `Type.Module`, `DynamicMethod(name, ..., Module, bool)`
  and `DynamicMethod.Module`.

Out, with the reason:

- **`AssemblyBuilder`/`TypeBuilder`/`ModuleBuilder`/`MethodBuilder`.** No rung in the plan 24 survey
  needs them (the only reach is `Expression.GetDelegateType` on byref delegate types, which
  slice 4 avoids). Reopen under a new plan only when a chosen application rung reaches
  `TypeBuilder`.
- **`DynamicILInfo`, `GetDynamicILInfo`, `GetTokenFor` and raw-IL `SetCode`.** These hand the VM IL
  bytes whose tokens index a `DynamicScope`, which the recording front end deliberately never
  builds. Supporting them needs a raw-IL decoder (dotnetdll 0.4.0's is private, so this means an
  upstream change or our own). The survey found no consumer: the libraries that emit use
  `GetILGenerator`. The stub throws `NotSupportedException` naming the API. Revisit only if a rung
  needs it.
- **Enabling `IsDynamicCodeCompiled`/`LambdaCompiler` as a default.** Slice 4 measures and records;
  a default changes only on host-rung evidence (plan 18 chose the current values that way).
- Debugger-facing and metadata-inspection members that no rung reads.

## Design

Keep the spike's architecture: the support-assembly stub records resolved operations, one
`InternalCall` hands the record to the VM, and the VM builds a one-method `dotnetdll`
`Resolution`, registers it through `AssemblyLoader::register_owned_assembly` and returns the
ordinary `MethodInfo`. Everything after that (dispatch, frames, delegates, exception regions,
`MemberRef` resolution, stack traces) is the existing path.

Match the CLR where managed code can observe it:

- `ILGenerator` follows CoreLib's `ILGenerator` contract: `DefineLabel`/`MarkLabel`,
  `BeginExceptionBlock` with the implicit `leave` and `endfinally`, `maxstack` from a stack-depth
  walk, and the same exception types for unmarked labels and unclosed blocks.
- The method is static and has no declaring type visible to reflection unless an `owner` is given.
  With an owner, the method runs with the owner's visibility (slice 3).

### Layering: the `dotnet-intrinsics-reflection` to `dotnet-assemblies` edge

The spike added `dotnet-assemblies` to `crates/dotnet-intrinsics-reflection/Cargo.toml`.
`docs/ARCHITECTURE.md` ("Trait Architecture") states one layering rule: intrinsic crates depend on
`dotnet-vm-ops`, never on `dotnet-vm`. The edge respects it and adds no cycle:

- `dotnet-vm-ops` already depends on `dotnet-assemblies` (`crates/dotnet-vm-ops/Cargo.toml`;
  `LoaderOps::loader` returns `&Arc<AssemblyLoader>`), and `dotnet-intrinsics-reflection` already
  depends on `dotnet-vm-ops`, so `dotnet-assemblies` was already in its transitive graph.
- `dotnet-assemblies` depends only on `dotnet-types`, `dotnet-utils` and `dotnet-value`, never on an
  intrinsic or VM crate. `dotnet-runtime-resolver` has the same direct edge.
- The crate uses only `AssemblyLoader` and `register_owned_assembly`, which `LoaderOps` already
  exposes through `ctx.loader()`.

Keep the direct edge. Naming `AssemblyLoader` in the builder's fields is its only purpose, and
re-exporting the type from `dotnet-vm-ops` would hide a real dependency. The architecture doc lists
crates flat, so it needs no change.

## Slices

Ordered by demand (STJ and the Linq thunk are the enabled paths that reach `DynamicMethod`), then
dependency. Each slice keeps the spike fixtures and both host rungs green.

1. **Complete the instruction set.** The spike landed with plan 24 and its fixtures are in
   `fixtures/reflection/`. Implement labels and all branch opcodes (short and
   long), `ldfld`/`stfld`/`ldsfld`/`stsfld`/`ldflda` with `FieldInfo` operands, `calli` with a
   signature operand, filter and fault blocks (first check that dotnetdll's `ExceptionKind` covers
   both), the full opcode table, and computed `maxstack`. Differential fixtures: a loop with a
   branch, a field read and write, a `calli`, a filter. Update the limits in the
   header comments as they shrink.
2. **Generic and nested types and callees, byref signatures.** Extend the signature translator in
   `dynamic_method.rs` to generic instantiations, nested types, generic method callees and byref
   parameters and returns. STJ's accessors and the Linq byref thunk need all four. Fixtures: a call
   to `List<int>.Add`, a generic method through `MakeGenericMethod`, a nested type's constructor,
   a `ref int` parameter written back through `Invoke`.
3. **`Module`, owner and `skipVisibility`; the rest of the stub's API.** Give `Type.Module`,
   `DynamicMethod.Module` and the `(..., Module, bool)` overload a real object, and implement
   `GetParameters`, `MethodHandle`, `ReturnTypeCustomAttributes` and `Define*Parameter`. First
   check whether the VM enforces member visibility at `call`/`ldfld` at all (the spike did not).
   If it does not, a fixture against `dotnet` records whether a non-`skipVisibility` method
   touching a private member throws on the CLR and whether matching that needs new enforcement.
4. **Retire the per-library seams and cover byref delegates.** With 1-3 in place:
   - Remove the `DefaultJsonTypeInfoResolver.get_MemberAccessor` seam and
     `DotnetRs.JsonReflectionMemberAccessor`, so STJ runs its own `ReflectionEmitMemberAccessor`
     (its `IsDynamicCodeSupported` check then matches the CLR). Keep the seam only if the STJ
     fixtures fail, and record why.
   - `CreateObjectArrayDelegate` for byref, pointer and more-than-16-parameter delegate types
     uses the CLR's `DynamicMethod` thunk, with byref arguments written back in a `finally`. This
     removes the `PlatformNotSupportedException` cases. Keep the generic thunk table for the
     shapes it handles unless a measurement says otherwise.
   - `IsDynamicCodeSupported` stays true (EF's NativeAOT check, plan 18). Experiment with
     `IsDynamicCodeCompiled` true and with `CanCompileToIL` true, and record the Newtonsoft, DI, EF
     InMemory and expression-fixture results here. A feature value or seam changes only when every
     host rung still matches `dotnet` on stdout and exit code in the release CLI. Otherwise record
     why it stays (for example `LambdaCompiler` needs closure types built with `TypeBuilder`,
     which is out of scope).
5. **Lifetime, stack traces, GC and threads.** Make compiled dynamic methods collectible: tie the
   synthetic resolution to the delegate or `DynamicMethod` object's lifetime and evict it from the
   metadata arena and the descriptor and method-info caches
   (`docs/TYPE_RESOLUTION_AND_CACHING.md`) once unreachable. Replace the `NEXT_DYNAMIC_ASSEMBLY`
   naming with one that survives eviction. Compare stack-trace frames (method name and
   `DeclaringType`) with `dotnet` and match. Add a concurrent-compile fixture under
   `multithreading` and a fixture that keeps a delegate alive across `GC.Collect`.

Slices 1-3 do not depend on the seam decision and are each useful alone. If slice 1 finds that
dotnetdll cannot express a required construct, stop and ask the maintainer rather than building a
decoder.

### Slice 1: upstream prefix and writer fixes

dotnetdll 0.5.0 added `CallVirtualConstrainedTail`, resolving the earlier `constrained.` + `tail.` +
`callvirt` gap. The promoted `reflection/dynamic_method_constrained_tail_42` fixture now matches
.NET 10.0.14.

A second valid combination, `constrained.` + `tail.` + `call` to a static abstract interface
method, returns 42 on .NET in `reflection/dynamic_method_static_constrained_tail_42`.
dotnetdll 0.5.1 added `CallConstrainedTail`, and the fixture now matches .NET.

A broader [0.5.0 audit](evidence/25-dotnetdll-050-audit.md) records reproducible branch,
signature, exception-section, and prefix-flag issues for the same upstream release. Several affect
serialized assemblies rather than this plan's in-memory `Resolution` path. The audit reproductions
were rerun against 0.5.1: the affected cases now round-trip, and the signature reader no longer
panics on the formerly truncated output.

The CLR accepts local-only `pinned` markers and superfluous sentinels in method signatures.
dotnetdll 0.5.1 cannot retain these nonstandard bytes in a resolved parameter. The recorder
preserves them in `SignatureHelper.GetSignature()` and normalizes them for standard managed calls.
Differential fixtures cover pinned arguments and sentinels; a CLR compacting-GC probe observed no
pinning effect for a reference argument in its tested cases. Exact metadata inspection is outside
this plan. See the [0.5.1 signature-edge probes](evidence/25-dotnetdll-051-signature-edges.md).

The recorder now covers branches, fields, filters, faults, computed stack depth, managed and
unmanaged indirect calls, basic `SignatureHelper` operands, and the standard opcode table. The
differential fixtures compare stdout and exit codes with .NET. Newtonsoft and EF InMemory host
rungs also match. Short branch targets are checked against the CLR's signed-byte displacement
limits before the recorded instructions are translated. Remaining limits include generic, nested
and byref signatures (slice 2), raw signature encoding for nonprimitive types, and uncommon `no.`
prefix combinations. On the tested
Linux CLR, invoking a managed vararg call throws
`InvalidProgramException`; the VM does the same. Managed vararg behavior on platforms where the
CLR supports it has not been implemented or tested.

Slice 1 verification (2026-11-15): all 55 DynamicMethod fixture and differential tests passed
both without features and with `multithreading`. The Newtonsoft and EF InMemory host projects
matched .NET on stdout and exit code with the multithreaded debug and release CLI.
`./check.sh --fast` passed clippy and 1123, 1429 and 1446 tests in its three configurations;
`cargo fmt --all -- --check` passed.

## Done when

- The slice fixtures match `dotnet` in both base feature sets (`multithreading` on and off).
- A POCO round-trip through System.Text.Json runs with no VM seam and matches `dotnet`.
- A delegate type with a byref parameter, created through the Linq path, matches `dotnet`.
- This plan records, with host-rung results, whether `IsDynamicCodeCompiled` and `CanCompileToIL`
  changed and why.
- Compiled dynamic methods that become unreachable are evicted, and a stack trace through one
  matches `dotnet`.
- The Newtonsoft roundtrip and EF InMemory rungs match `dotnet` on stdout and exit code, and
  `./check.sh --fast`, clippy `-D warnings` and fmt pass.

## Ask the maintainer only if

- A needed construct (filter or fault clause, a signature operand, a `calli` shape) cannot be
  written with dotnetdll 0.4.0's resolved writer, so an upstream change or a raw-IL decoder would be
  required.
- Honoring `owner`/`skipVisibility` would require member-visibility enforcement the VM does not have
  today.
- A host rung stops matching `dotnet` after a slice and the cause is the stub replacing
  `DynamicMethod` for every caller.
- A chosen rung needs `TypeBuilder` or `DynamicILInfo` (both are out of scope here).
