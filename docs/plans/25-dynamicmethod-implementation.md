# Implement `DynamicMethod`

**Status:** Complete except eviction of unreachable dynamic methods (plans README, "Not yet planned")
**Outcome:** `System.Reflection.Emit.DynamicMethod` runs on the VM for every `ILGenerator`
operation a real library uses: branches and labels, fields, `calli`, filters and faults, generic
and nested types and callees, byref signatures, and the owner, module and `skipVisibility`
overloads. System.Text.Json serializes a POCO through its own `ReflectionEmitMemberAccessor` (no VM
seam), the Linq object-array delegate path handles byref and pointer delegate types, and the
differential fixtures match `dotnet`. The Newtonsoft and EF InMemory host rungs still match.

Follows the spike of [plan 24](24-reflection-emit-investigation.md).

## Design

The support assembly's `DotnetRs.DynamicMethod` (`crates/dotnet-assemblies/src/support/DynamicMethod.cs`)
replaces `DynamicMethod` for every caller. Its `ILGenerator` records resolved operations
(opcode, immediate, operand) following CoreLib's `ILGenerator` contract: `DefineLabel`/`MarkLabel`,
`BeginExceptionBlock` with the implicit `leave` and `endfinally`, the same exceptions for unmarked
labels and unclosed blocks, and short-branch targets checked against the signed-byte limit. The
`CompileRecorded` internal call hands the record to
`crates/dotnet-intrinsics-reflection/src/dynamic_method.rs`, which builds a one-method `dotnetdll`
`Resolution` (computing `maxstack` from a stack-depth walk), registers it with
`AssemblyLoader::register_owned_assembly` and returns an ordinary `MethodInfo`. Dispatch, frames,
delegates, exception regions, `MemberRef` resolution and stack traces then use the existing paths.

- Each synthetic method has its own metadata arena and a runtime marker, so reflection reports a
  null `DeclaringType` (unless an owner is given) and stack traces show the method name without a
  synthetic holder. Assembly names come from a checked, process-wide 64-bit counter
  (`DYNAMIC_ASSEMBLY_ID`) that never reuses or wraps.
- Signatures cover generic instantiations, nested types (referenced through their enclosing
  types), generic method callees, and byref parameters and returns.
- `Type.Module` returns a stable manifest-module object per loaded assembly; owner- and
  module-hosted methods report it, and anonymous methods share a separate module.
- STJ runs its own `ReflectionEmitMemberAccessor`; synthetic methods named `.ctor` are classified by
  their runtime-special-name flag. The Linq generic thunk table stays for the shapes it handles;
  byref, pointer and more-than-16-parameter delegate types use the framework's
  `CreateObjectArrayDelegateRefEmit` over `DynamicMethod`.
- The `dotnet-intrinsics-reflection` → `dotnet-assemblies` dependency edge is deliberate: it adds
  no cycle (`dotnet-vm-ops` already depends on `dotnet-assemblies`) and names `AssemblyLoader`
  directly instead of hiding the dependency behind a re-export.

### Dynamic-code switches

`IsDynamicCodeSupported` stays true (EF Core treats false as NativeAOT). Release-CLI experiments
(multithreading, .NET 10.0.14, stdout and exit code):

| Configuration | Newtonsoft | DI | EF InMemory | Expression samples |
| --- | --- | --- | --- | --- |
| `IsDynamicCodeCompiled` true | Fail: `DynamicMethod.set_InitLocals` missing | Pass | Pass | Pass |
| `CanCompileToIL` true | Pass | Pass | Fail: `RuntimeType.GetMembers` missing | Pass |
| Both unchanged | Pass | Pass | Pass | Pass |

Both switches stay false because each experiment failed a host rung. The DI rung's matching output
does not prove that the provider's background compilation succeeded, because DI catches
compilation errors. The host projects are `crates/dotnet-cli/tests/host_projects/newtonsoft_roundtrip`
and `dependency_injection` and `crates/dotnet-benchmarks/fixtures/ef_inmemory`; run them with
`scripts/diff_run.sh --host` and `DOTNET_RS_BIN` set to the release CLI.

### dotnetdll

dotnetdll 0.5.1 added `CallVirtualConstrainedTail` and `CallConstrainedTail`
(`dynamic_method_constrained_tail_42`, `dynamic_method_static_constrained_tail_42`) and fixed the
branch, signature, exception-section and prefix-flag issues found in a 0.5.0 audit. It cannot keep
nonstandard signature bytes (local-only `pinned` markers, extra sentinels) in a resolved parameter:
the recorder preserves them in `SignatureHelper.GetSignature()` and normalizes them for standard
managed calls ([0.5.1 signature-edge probes](evidence/25-dotnetdll-051-signature-edges.md)).

## Limits and out of scope

- **`AssemblyBuilder`/`TypeBuilder`/`ModuleBuilder`/`MethodBuilder`**: not supported (plan 24).
- **`DynamicILInfo`, `GetDynamicILInfo`, `GetTokenFor` and raw-IL `SetCode`** throw
  `NotSupportedException` naming the API: they would need a raw-IL decoder, and no surveyed
  library uses them.
- **Managed vararg calls** throw `InvalidProgramException`, as the Linux CLR does; vararg behavior
  on platforms where the CLR supports it is neither implemented nor tested.
- **Visibility.** `owner` and `skipVisibility` are not enforced, because the VM's `call` and field
  paths enforce no general member visibility. On the .NET 10.0.14 Linux CLR the tested private
  calls and fields also work with `skipVisibility` false.
- Raw signature encoding for nonprimitive types and uncommon `no.` prefix combinations.
- `MethodHandle` throws `InvalidOperationException`, as on .NET.

The vararg and visibility limits are tracked in the plans README, "Not yet planned".

## Eviction analysis

Compiled dynamic methods and their metadata arenas are never reclaimed. Safe eviction needs every
owner represented in the reachability graph before an arena is released:

- Delegates and `DotnetRs.ParameterInfo` retain only a method index, and `ldftn` and method-handle
  function pointers expose indexes as native integers, so exposed function pointers need a way to
  retain their target.
- Reflection tables root method objects in each GC mode, and the shared table can root objects
  from another arena; the loader and reflection registries keep strong references to generated
  assemblies and method objects; method-info and descriptor caches retain metadata.
- Registries and caches must be pruned after cross-arena marking and finalizer promotion; active
  frames must retain their descriptors; synthetic resolutions referenced by other synthetic methods
  must stay alive; resurrection must be handled.
- `ResolutionS::definition` and descriptor accessors return `'static` metadata references that can
  outlive an evicted arena; that lifetime contract must be repaired first.

Removing table entries based only on a dead `DynamicMethod` or delegate would leave live indexes
dangling. `dynamic_method_gc_42` proves survival across collection, not reclamation.

## Checks

The `crates/dotnet-cli/tests/fixtures/reflection/dynamic_method_*_42.cs` differential fixtures
(including `dynamic_method_stack_trace_42` and `dynamic_method_gc_42`) in both base feature sets,
`threading/dynamic_method_concurrent_42.cs` under `multithreading`, the STJ POCO round-trip
(`json_serializer_poco_42`), the expression fixtures, and the three host rungs.
