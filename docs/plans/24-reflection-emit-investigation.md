# Investigate Reflection.Emit and `DynamicMethod`

**Status:** Complete. Slices 1-5 done; the decision is a follow-up `DynamicMethod` implementation via a synthetic module, [plan 25](25-dynamicmethod-implementation.md). The spike landed with this plan (see [Landing](#landing)).
**Outcome:** This plan records a decision, backed by a working spike or a measured blocker:
either a follow-up plan to implement `DynamicMethod` (and, separately, whether to do
`AssemblyBuilder`/`TypeBuilder`), or a single switch-level answer that replaces the per-call-site
patches. The `RuntimeType.GetNativeTypeHandle` access violation is either fixed or reproduced
with its cause recorded.

Closes these README "Not yet planned" entries once the decision is made: **Dynamic code without
Reflection.Emit** (replaced by the follow-up plan, or closed by the switch-level answer) and
**Access violation in `RuntimeType.GetNativeTypeHandle`**.

## Problem

- The VM reports `RuntimeFeature.IsDynamicCodeSupported` true (EF Core treats false as NativeAOT)
  and `IsDynamicCodeCompiled` false (`crates/dotnet-vm/src/intrinsics/app_context.rs`, plan 18
  slice 7). Code that checks only `IsDynamicCodeSupported` before emitting IL fails.
- Two such paths are patched per call site in `crates/dotnet-vm/src/stack/call_ops_impl.rs`:
  `try_redirect_expression_compile_to_interpreter` (redirects `LambdaExpression.Compile()` to the
  `preferInterpretation` overload) and `try_no_body_runtime_stub` (stubs
  `DelegateHelpers.<CreateObjectArrayDelegateRefEmit>g__ForceAllowDynamicCode`). Each new library
  that emits IL would need another patch.
- When Newtonsoft used Reflection.Emit, `DynamicMethod.Init` → `get_IsActualInterface` →
  `RuntimeType.GetNativeTypeHandle` raised an `AccessViolationException`. No rung reaches it now
  ([evidence](evidence/18-array-casts-host-rungs.txt)), but managed code should never produce
  one.
- The VM's method model may not fit runtime-created methods. `MethodDescription`
  (`crates/dotnet-types/src/members.rs`) is a (`ResolutionS`, `MethodMemberIndex`) pair into
  loaded dotnetdll metadata, and bodies are decoded by dotnetdll with tokens resolved against that
  module's tables. A `DynamicMethod` body's tokens instead index into the `DynamicResolver`'s
  object list (`DynamicILGenerator` / `DynamicScope`), and it has no owning metadata row.

## Questions to answer

1. **Demand.** Which real code paths reach Emit today or would with the patches removed? Check
   Newtonsoft, EF Core InMemory, Microsoft.Extensions.DependencyInjection, System.Text.Json,
   `System.Linq.Expressions` and EF Core SQLite (the next candidate rung). For each, record:
   whether it checks `IsDynamicCodeSupported` or `IsDynamicCodeCompiled`, and whether a
   non-Emit fallback exists.
2. **Switch-level alternative.** Is there one AppContext switch, feature value or CoreLib
   substitution (for example making `DynamicMethod`'s constructor throw `PlatformNotSupportedException`
   the way NativeAOT's CoreLib does, which libraries catch) that makes every path in (1) take its
   fallback without breaking EF's NativeAOT check? Test it by removing both call-site patches.
3. **`DynamicMethod` feasibility.** What is the smallest VM change that runs a `DynamicMethod`
   body?
   - **(a) Token-resolver body source:** a new `MethodDescription` variant or side table keyed by
     a VM-allocated handle, with a body decoder that resolves tokens through the managed
     `DynamicResolver` (the CLR's `DynamicResolver.ResolveToken`/`GetCodeInfo` contract).
   - **(b) In-memory assembly:** build a one-method module with dotnetdll's writer, rewriting
     tokens to MemberRefs into the referenced assemblies, and load it as an ordinary `ResolutionS`.
   For each option, estimate the touched modules (resolver, type caches, call dispatch, stack
   traces, exception handling regions, `RuntimeMethodHandle` and reflection objects, GC of
   collectible methods) and note whether the existing intrinsic dispatch, effects and frame homes
   work unchanged.
4. **`AssemblyBuilder`/`TypeBuilder`.** Does any rung in (1) need full type emission, or only
   `DynamicMethod`? Size it only as far as needed to decide whether it belongs in the follow-up.
5. **Access violation.** Reproduce `DynamicMethod.Init` → `get_IsActualInterface` → `GetNativeTypeHandle`
   with a direct fixture (`new DynamicMethod("m", typeof(int), Type.EmptyTypes)`). Find which
   handle or pointer is invalid. Fix it if the fix is local to type handles (it may affect other
   reflection paths), otherwise record the cause.

## Slice 1 findings: demand survey

Surveyed 2026-11-14 against the versions the rungs resolve (Newtonsoft.Json 13.0.3 `net6.0`,
EF Core + InMemory + Relational + SQLite 9.0.17, Microsoft.Extensions.DependencyInjection 9.0.17
`net9.0`, and System.Text.Json and System.Linq.Expressions from the 10.0.14 shared framework).
Sources were read with `ilspycmd`. Runtime evidence comes from method traces of both host rungs
and three probe programs; see [evidence](evidence/24-demand-survey.txt).

| Library | Selects Emit with | Non-Emit fallback | Reaches Emit on the VM today |
|---|---|---|---|
| Newtonsoft.Json | `IsDynamicCodeCompiled` (`JsonTypeReflector.DynamicCodeGeneration`) | `LateBoundReflectionDelegateFactory`, `ExpressionValueProvider` | No. The rung compiles 4 lambdas, all 1–2 parameters |
| M.E.DependencyInjection | `IsDynamicCodeCompiled && !DisableDynamicEngine` (`ServiceProvider.GetEngine`) | `RuntimeServiceProviderEngine` | No |
| EF Core (+ InMemory) | No direct Emit. About 26 `LambdaExpression.Compile()` sites, and its `IsDynamicCodeSupported` check only refuses model building when false | Linq interpreter (see below) | No on the rung: 56 lambdas compiled, all within the thunk limit. Yes for 3-parameter lambdas, e.g. `DependencyInjectionMethodParameterBinding`, `ServiceParameterBinding` |
| EF Core Relational / SQLite | No direct Emit. Query shapers are 3–4-parameter lambdas (`QueryContext, DbDataReader, ResultContext, ResultCoordinator`); split-include loaders take 3 | Linq interpreter | Would, on every query. The probe stops earlier on an unrelated gap (below) |
| System.Text.Json | `IsDynamicCodeSupported` (`DefaultJsonTypeInfoResolver.MemberAccessor` picks `ReflectionEmitCachingMemberAccessor`) | `ReflectionMemberAccessor`, chosen only when `IsDynamicCodeSupported` is false | Would, for every reflection-based POCO. The probe stops earlier on an AV (below) |
| System.Linq.Expressions | `CanCompileToIL` = `IsDynamicCodeSupported`; the interpreter separately uses `CanEmitObjectArrayDelegate` | `LightCompiler`, but see below | Yes, for any delegate with more than 2 parameters or with byref/pointer types |

**The interpreter still emits.** The `LambdaExpression.Compile()` redirect avoids `LambdaCompiler`,
but every interpreted lambda becomes a delegate through `LightLambda.MakeDelegate` →
`DelegateHelpers.CreateObjectArrayDelegate`. Its `CanEmitObjectArrayDelegate` feature switch is a
trimmer-substituted IL constant `true`, not an AppContext read. `CreateObjectArrayDelegateRefEmit`
avoids Emit only when `GetCSharpThunk` matches, which needs at most 2 parameters and no
byref, byref-like or pointer types. Anything wider constructs a `DynamicMethod`. The non-Emit
branch calls `Internal.Runtime.Augments.DynamicDelegateAugments.CreateObjectArrayDelegate` via
`UnsafeAccessor`, and that type exists only in NativeAOT's CoreLib, not in the CoreCLR CoreLib
the VM loads. Two consequences:

- A switch-level answer (question 2) must also cover this path, for example a VM-provided
  `CanEmitObjectArrayDelegate` = false plus an intrinsic `CreateObjectArrayDelegate` that builds a
  delegate packing its arguments into `object[]`. Feature values alone can't do it.
- `Expression<Func<int,int,int,int>>.Compile()` fails on the VM today
  (`TypeInitializationException`), even through the redirect.

**Question 5 at survey time** (corrected in slice 2). The bare
`new DynamicMethod("m", typeof(int), Type.EmptyTypes)` takes `Init`'s `transparentMethod` branch,
which never checks `IsActualInterface`. It fails in `GetDynamicMethodsModule` →
`AssemblyLoadContext.get_Default` → `DefaultAssemblyLoadContext..cctor` →
`InitializeAssemblyLoadContext` with `DllNotFoundException: Unable to load DLL 'QCall'`, surfacing
as `TypeInitializationException`. The AV that Newtonsoft hit came from the owner-type overload. A
second AV path was System.Text.Json's `ObjectConverterFactory.CreateConverter` →
`Activator.CreateInstance` → `RuntimeType.CreateInstanceImpl` → `GetConstructorCandidates` →
`RuntimeType.get_Cache()`.

**EF Core SQLite's first wall isn't Emit.** `new SqliteConnection("Data Source=:memory:")`
throws `ArgumentException: Connection string keyword 'data source' is not supported` on the VM,
while `dotnet` accepts it. This is likely a case-insensitive keyword lookup gap and belongs in the README
"Not yet planned" list, not in this plan.

**Answer to question 1.** Real demand exists. Every library that selects on `IsDynamicCodeCompiled`
already avoids Emit. The remaining demand comes from two sources: System.Text.Json, which selects
on `IsDynamicCodeSupported`, and the Linq interpreter's delegate creation for lambdas with more
than 2 parameters, which EF Relational/SQLite hits on every query. Neither has a fallback that
current feature values can reach. The first maintainer question ("no real-application path needs
Emit") does not apply.

## Slice 2 findings: the access violation

Done 2026-11-14 ([evidence](evidence/24-demand-survey.txt), "Slice 2").

**Cause.** Every `System.RuntimeType` object on the VM is a `DotnetRs.RuntimeType` (the support stub
that replaces it), whose only field is the type index. CoreLib methods the stub doesn't replace
still run their own IL. When that IL reads a CoreLib-only field (`m_handle` through
`GetNativeTypeHandle`, `m_cache` through `RuntimeType.Cache`), the field offset lies outside the
stub object, the bounds-checked read fails, and `ldfld` reports `AccessViolationException`. Virtual
calls already reach stub overrides. Definition-bound calls inside CoreLib (`call`/`callvirt` to a
non-virtual member through a MethodDef token) went to CoreLib's body. Both reported paths are
of this kind:

- `DynamicMethod(name, returnType, parameterTypes, Type owner, bool skipVisibility)` →
  `RuntimeType.IsActualInterface` → `GetNativeTypeHandle` (`m_handle`). This is the path Newtonsoft's
  `DynamicReflectionDelegateFactory` took.
- `Activator.CreateInstance(Type, BindingFlags, Binder, object[], CultureInfo[, object[]])` →
  `RuntimeType.CreateInstanceImpl` → `GetConstructorCandidates` → `Cache` (`m_cache`), which is how
  System.Text.Json creates its object converters.

**Fix** (local to `RuntimeType`):

- `crates/dotnet-assemblies/src/resolution.rs` sends definition-bound calls to `System.RuntimeType`
  members to the stub when the stub implements them in IL, the same way `Array.Copy` was already
  redirected. Stub members that are `InternalCall` externs keep their existing routing.
- The stub implements `IsActualInterface` (as `IsInterface`) and `CreateInstanceImpl`. The latter
  mirrors CoreLib's body, including `FilterApplyMethodBase`'s arity and `params` rules, the
  `DefaultBinder` binding, value-type defaults and `DoNotWrapExceptions`, but enumerates
  constructors with `GetConstructors`.
- Two gaps the new path exposed are fixed: `MethodBase.CallingConvention` had a registered
  intrinsic with no handler (it panicked), and `Type.IsGenericParameter` wasn't reported, so
  `ContainsGenericParameters` was false for open generics.
- `ldfld`/`stfld` now check, on their failure path only, whether the object is a support stub
  standing in for the field's declaring type. If so they stop with a VM error naming the field,
  the stub and the calling method, for example
  `System.RuntimeType.m_cache is not modeled by support stub DotnetRs.RuntimeType (accessed from
  System.RuntimeType.get_Cache)`, instead of raising an `AccessViolationException` that managed
  code could catch. Any remaining member of this family now names the method to replace.

**Fixtures.** `reflection/activator_create_instance_binding_flags_42`,
`reflection/dynamic_method_owner_validation_42` (interface, array and open generic owners are
rejected with `ArgumentException`; the interface cases raised the AV before the fix) and
`reflection/is_generic_parameter_42` pass against `dotnet`.
`debug_fixtures/dynamic_method_ctor_42` stayed out of the suite until `DynamicMethod` worked; it
moved to `fixtures/reflection/` when the spike landed (see [Landing](#landing)).

**Next walls, all Emit-side** (for slices 3–4):

- A bare `DynamicMethod` (and every Linq interpreter thunk wider than 2 parameters) stops at the
  `InitializeAssemblyLoadContext` QCall from `GetDynamicMethodsModule`.
- An owner-type `DynamicMethod` with a valid owner stops at `RuntimeType.GetRuntimeModule` →
  `RuntimeTypeHandle.GetModuleIfExists` (no FCall).
- System.Text.Json now reaches `ReflectionEmitMemberAccessor.CreateParameterlessConstructor`, which
  stops at `Type.Module` (`DotnetRs.RuntimeType.GetModule` is an extern with no intrinsic). The VM
  has no module reflection object (`DotnetRs.Module` is empty).

So every `DynamicMethod` entry point first needs a `Module`/`RuntimeModule` the VM can hand out.
Slice 4 should count that toward both options in question 3.

**Not fixed here:** `ParameterInfo` reports no custom attributes, so `ParamArrayAttribute` is
invisible and `params` constructors don't bind through `CreateInstanceImpl` (logged in the README
"Not yet planned").

## Slice 3 findings: switch-level experiment

Done 2026-11-14 on `plan24/slice3-switch`, with both call-site patches deleted from
`call_ops_impl.rs` ([evidence](evidence/24-demand-survey.txt), "Slice 3").

**What works.** Three intrinsics in `crates/dotnet-vm/src/intrinsics/linq_expressions.rs`, plus two
small support classes, replace both patches and make every Linq path avoid Emit:

- `LambdaExpression.get_CanCompileToIL` returns false. `Compile()` then takes `LightCompiler`
  without the `Compile(bool)` redirect, the branch NativeAOT takes. The same property also routes
  `Expression.Lambda<T>`/`CreateLambda` to `ExpressionCreator<T>` (plain managed),
  `MakeUpdateDelegate` to `CreateCustomUpdateDelegate` and `GetDelegateType` to the VB-style call
  site. These are the same branches NativeAOT takes, and EF Core doesn't read `CanCompileToIL`.
- `DelegateHelpers.get_CanEmitObjectArrayDelegate` returns false (the value NativeAOT's trimmer
  substitutes), and `DelegateHelpers.CreateObjectArrayDelegate(Type, Func<object[], object>)`
  runs `DotnetRs.ObjectArrayDelegates.Create`. That stands in for NativeAOT's
  `DynamicDelegateAugments.CreateObjectArrayDelegate`. It reuses the existing mechanism: the
  support class has generic `ActionThunk0..16`/`FuncThunk0..16` (each packs its arguments into an
  `object[]`, calls the handler and casts the result, the same shape as the Emit thunk), and
  `Create` picks one by `MakeGenericMethod` and binds it with `CreateDelegate(delegateType,
  handler)`. This is the path CoreLib already takes for 0–2 parameters, so it needs no new VM
  delegate machinery. The intrinsic only locates the support method and dispatches to it with the
  arguments already on the stack. The `ForceAllowDynamicCode` stub is gone because nothing reaches
  `CreateObjectArrayDelegateRefEmit` any more.
- `DefaultJsonTypeInfoResolver.get_MemberAccessor` runs `DotnetRs.JsonReflectionMemberAccessor.Get`,
  which instantiates System.Text.Json's own `ReflectionMemberAccessor` (the VM passes the `Type`
  object because `Type.GetType(string)` doesn't work yet). This is a per-library seam, not a
  switch; see the answer below.

About 100 lines of Rust and 160 lines of C# (mostly the thunk table). No dotnetdll change.

**Limits of the thunk table.** Delegates with byref, pointer or byref-like parameters or return
types, and delegates with more than 16 parameters, throw `PlatformNotSupportedException`.
The CLR's Emit thunk supports them (byref arguments are written back in a `finally`). A byref
thunk can't be written as a generic C# method, so it needs either a VM-built delegate whose
`Invoke` dispatches to an intrinsic or real `DynamicMethod` support. Nothing in the rungs needs
it; `Expression.GetDelegateType` on byref types still ends in `TypeBuilder`
(`MakeNewCustomDelegate`, gated by `IsDynamicCodeSupported`) and fails for the same reason it did
before.

| Target | A. `CanEmit` false + `CreateObjectArrayDelegate` intrinsic, `CanCompileToIL` untouched | B. A + `CanCompileToIL` false | C. diagnostic only: `IsDynamicCodeSupported` false | D. B + STJ `MemberAccessor` seam (kept) |
|---|---|---|---|---|
| 3-parameter lambda probe | FAIL: `Compile()` reaches `LambdaCompiler` and `DynamicMethod.Init` | PASS | not run | PASS |
| Expression fixtures (`expression_compile_42`, `expression_call_object_hashcode_42`) | not run | PASS | not run | PASS |
| New fixtures (`expression_compile_many_params_42`, `expression_custom_delegate_42`) | not run | PASS | not run | PASS |
| Newtonsoft roundtrip | not run | PASS | PASS | PASS |
| EF InMemory | not run | PASS | FAIL: `NativeAotNoCompiledModel` | PASS |
| STJ POCO probe | not run | stops at `Type.Module` | PASS | PASS |
| `json_serializer_poco_42` fixture | n/a | n/a | n/a | PASS |

Removing the patches with nothing added was not run on its own: attempt A shows why it can't work
(`Compile()` goes to `LambdaCompiler` once the redirect is gone). C is a throwaway intrinsic that
confirms plan 18's finding about EF and shows that STJ's fallback runs on the VM.

**Answer to question 2.** Partly. There is no single AppContext switch or feature value, because
the demand comes from three different mechanisms:

- *Linq: covered by switch-shaped substitutions.* `CanCompileToIL` and `CanEmitObjectArrayDelegate`
  are exactly the two switches NativeAOT flips, and they let the call-site patches go with
  `IsDynamicCodeSupported` still true. The missing piece was NativeAOT's
  `DynamicDelegateAugments.CreateObjectArrayDelegate`, which the VM now supplies from the support
  assembly. Every interpreted lambda up to 16 parameters works, including the 3–4-parameter shaper
  lambdas EF Relational compiles on every query. Byref/pointer delegate types and more than 16
  parameters are the uncovered part.
- *System.Text.Json: not covered by any switch.* It chooses its accessor from
  `IsDynamicCodeSupported` alone, with no feature switch of its own, and EF's check rules out
  flipping that. The VM can still serve the fallback with one per-library seam
  (`get_MemberAccessor`), after which `Serialize`/`Deserialize` of a POCO matches `dotnet`.
  That is an STJ-specific intercept, so it isn't the "one answer for every library" the question
  asked for; a library that picks Emit the same way needs its own seam.
- *`DynamicMethod` throwing `PlatformNotSupportedException`: rejected.* No library that reaches
  `DynamicMethod` catches it. STJ's `ReflectionEmit*MemberAccessor` and the Linq thunk call
  `new DynamicMethod` unguarded; Newtonsoft only emits behind `IsDynamicCodeCompiled`; EF has no
  Emit path; the one `NotSupportedException` catch in System.Linq.Expressions
  (`CallInstruction.Create`) surrounds `MakeGenericType`/`CreateDelegate`, not `DynamicMethod`.
  Throwing would turn today's missing-QCall failure into a different exception at the same sites.
  NativeAOT works because its libraries check the feature flags first, which is what the
  substitutions above reproduce.

So the call-site patches are removable now, and the switch-level approach covers everything the
rungs reach. What it leaves is the set of libraries that emit on `IsDynamicCodeSupported` without
a switch (STJ is the only one found), each needing its own seam, plus byref delegate types. Whether
that favors a real `DynamicMethod` (slice 4) is a call for the decision slice: the seam approach
stops being cheap once a library's fallback isn't a drop-in object.

**Unrelated gaps found while writing the fixtures** (none touch Emit; logged in the README "Not yet
planned" list):

- `"true"u8` static spans crash the VM (`Utf8JsonWriter` writing a bool or null segfaults).
  *Fixed after slice 3:* `RetainedPtr::checked_range` returned a static-origin pointer's null cached
  address to `Buffer.Memmove` and never bounds-checked heap ranges; it now resolves and checks
  every owned origin.
- `JsonSerializer.Deserialize` reads numbers wrongly: a `double` followed by more members is 0 every
  run, and an `int` in a nested object was 0 in about 1 run in 25. *Fixed after slice 3:* the
  cause was the property-name lookup, not number parsing. VM strings had no NUL terminator, so
  `String.GetNonRandomizedHashCode` read past odd-length names ("Score", "Zip"), the string and
  span hashes disagreed, and `Dictionary` alternate lookups missed. The VM's
  `GetHashCodeOrdinalIgnoreCase` intrinsic had the same kind of mismatch and was removed.
- Expression trees built from C# that capture a local or set a field through an object initializer
  stop in `FieldInfo.GetFieldFromHandle` (`ldtoken` of a field).
- `Type.GetType(string)` and `Assembly.GetType(string)` hit the `QCall` DLL error.

## Slice 4 investigation: dynamic method entry points

### Baseline investigation (before the spike)

2026-11-15. At this point the execution spike did not yet work. Three independent
fixtures are in `crates/dotnet-cli/tests/debug_fixtures`: `dynamic_method_add_42` checks both
`Invoke` and `CreateDelegate`, `dynamic_method_call_42` emits a call to an existing method, and
`dynamic_method_catch_42` emits a constructor call, throw and typed catch. On .NET 10.0.14 all
three print 42 and exit 42. On the unmodified VM all three fail in the constructor at the
`DefaultAssemblyLoadContext` initialization QCall, before emitting any IL.

A diagnostic experiment replaced `GetDynamicMethodsModule` with a freshly allocated, uninitialized
CoreLib `RuntimeModule` (14 lines), and then replaced `RuntimeType.GetCorElementType` with the
constant for `Int32` (11 lines). These substitutions are deliberately invalid for general use and
were removed after the experiment. With only the first substitution, the add fixture constructs the
method but `GetILGenerator` reaches `SignatureHelper.AddOneArgTypeHelperWorker` →
`RuntimeType.GetCorElementType` → `GetNativeTypeHandle`, which tries to read `m_handle` from the
`DotnetRs.RuntimeType` stub. With both substitutions, the add fixture emits all four instructions
and reaches `ModuleHandle.GetDynamicMethod`'s QCall through `Invoke`; it fails there with
`DllNotFoundException`. The other two fixtures reach their `Emit` calls, but CoreLib's
`DynamicILGenerator.Emit` requires a `RuntimeMethodInfo` or `RuntimeConstructorInfo`. The VM
returns `DotnetRs.MethodInfo` / `DotnetRs.ConstructorInfo` objects instead, so both throw
`ArgumentException` before adding a token. The command and diagnostic patch are preserved in
[evidence](evidence/24-slice4-probe.patch).

The next boundary is thus not just a module object: the VM needs a real module identity and
signature/type-handle operations; a way for Emit to accept VM reflection objects and populate
`DynamicScope`; and a `GetDynamicMethod` bridge that converts the resolver's code, locals, tokens
and exception regions into executable VM state. No emitted body, token resolution, exception
handling, `Invoke`, or dynamic delegate has been validated on the VM yet.

**Implementation comparison (estimates, not measured implementations).** Both options need the
module and reflection/Emit bridge above. In either, `DynamicResolver.GetCodeInfo` supplies raw IL,
max stack, init-locals and exception count; its locals signature and EH data need decoding, and
`DynamicScope` tokens resolve to managed handles or other objects, not module metadata rows.

- **(a) Resolver-backed method.** `MethodDescription` and `build_method_info` currently assume a
  metadata parent, `MethodIndex`, signature and body. A dynamic representation or side table would
  have to cover intrinsic classification, method-info and resolution caches, reflection registry,
  call/delegate dispatch and stack traces. The instruction and exception consumers expect
  dotnetdll's resolved instructions and instruction-indexed regions. Resolving managed tokens may
  require managed calls/effects and a traced lifetime for the resolver; it cannot be assumed that
  existing effect handling is unchanged. Once correctly materialized, ordinary frame homes and
  exception dispatch may be reusable, but this has not been tested. The metadata arena and cache
  currently retain descriptors; collectible methods need an explicit lifetime/eviction design.
- **(b) Synthetic module.** A generated type and method could retain the existing descriptor,
  method-info, intrinsic dispatch, stack trace, frame-home and EH paths after conversion. Building
  a `Resolution` with a resolved `body::Method` is supported; the existing fault fixture shows
  dotnetdll can write a method with calls and exception clauses. That does **not** establish that
  it can translate a dynamic body. dotnetdll 0.4.0 exposes binary IL but its binary-to-resolved
  conversion module is private and expects a metadata context. Its writer accepts resolved
  instructions, not `DynamicResolver` raw bytes. The spike must either implement the conversion
  and token rewriting, construct a valid PE with rewritten metadata and let dotnetdll parse it, or
  obtain a suitable upstream API. Method/type/field references, generic contexts, signatures,
  strings, locals and EH catch tokens all need translation; dynamically emitted methods calling
  one another and collectible methods need identities and lifetime handling. The existing loader
  only has a public file-based loading entry point; an in-memory registration path is also needed.

Neither option had yet established that an upstream dotnetdll change is necessary, or that option
(a) requires changing every resolver cache. The spike below answers both.

### Spike result (2026-11-15): all three fixtures pass

`dynamic_method_add_42` (both `Invoke` and `CreateDelegate`), `dynamic_method_call_42` and
`dynamic_method_catch_42` print 42 and exit 42, and `bash scripts/diff_run.sh` reports
`PASS: exit code and stdout match (mode=host)` for each. They stay in `debug_fixtures`; the
decision slice promotes them. `dynamic_method_ctor_42` (slice 2) also passes now. Existing suite:
`./check.sh --fast` passes (1068/1374/1391 tests across its three stages) and
`cargo clippy --workspace --all-targets -- -D warnings` is clean.

**Option chosen: (b), a synthetic module, with a different front end than the plan imagined.** The
spike does not run CoreLib's `DynamicILGenerator`/`DynamicScope`/`DynamicResolver` at all. It
replaces `System.Reflection.Emit.DynamicMethod` with a stub type in the support assembly
(`[Stub(InPlaceOf = ...)]`, the mechanism `System.Array` and `System.RuntimeType` already use). The
stub's `GetILGenerator()` returns an `ILGenerator` subclass that records `(opcode, immediate,
reference operand)` per instruction, plus locals and exception clauses, as instruction-indexed
data. On the first `Invoke`/`CreateDelegate`, one `InternalCall` hands the record to the VM, which
builds a `dotnetdll` `Resolution` with one type and one static method, registers it through the
existing `AssemblyLoader::register_owned_assembly`, and returns the ordinary
`DotnetRs.MethodInfo` for it. The shim then forwards `Invoke`/`CreateDelegate` to that
`MethodInfo`.

Why this beats the two options as written:

- **(a)** needs a new `MethodDescription` representation (`parent`, `method_resolution` and
  `method_member_index` all assume metadata) that every resolver cache keys on. That is an "Ask the
  maintainer" condition, so it was not attempted. It is still not measured.
- **(b) as written** needs `DynamicResolver` raw IL bytes decoded to resolved instructions.
  dotnetdll 0.4.0's decoder is private, so it would need an upstream change or a hand-written
  decoder. Recording resolved operations in the generator removes the decoder: dotnetdll's writer
  side takes resolved `Instruction` values directly, and a `Resolution` built in memory needs no
  PE round trip. **No dotnetdll change was needed.**
- It also removes the module identity, `GetCorElementType`/`GetNativeTypeHandle` and
  `DynamicScope`-accepts-VM-reflection-objects walls from the baseline investigation, because none
  of that code runs.

**Per-step record** (no earlier steps to compare against: each step was the first run of its
fixture; the shim and VM bridge were written together and the fixtures passed on the first run of
the complete bridge after the build errors below were fixed).

| Step | What broke | Fix | Code |
|---|---|---|---|
| Entry (add, ctor to emit) | CoreLib `DynamicMethod` ctor needs `DefaultAssemblyLoadContext` and a real `RuntimeModule` | Stub replaces the type; no CoreLib code runs | `DynamicMethod.cs`, 250 lines in the support assembly (stub, `ILGenerator` subclass, `LocalBuilder` subclass, owner/module constructor overloads) |
| Compile support assembly | In namespace `DotnetRs`, bare `MethodInfo`, `ConstructorInfo`, `FieldInfo`, `Delegate`, `RuntimeMethodHandle` bind to the support types, not `System.*` | Fully qualify | a handful of lines |
| VM bridge (add) | `no body in executing method: ...CompileRecorded` (the expected next boundary) | Intrinsic `CompileRecorded` | `dynamic_method.rs`, 528 lines in `dotnet-intrinsics-reflection`, plus 1 line in `lib.rs` and a `dotnet-assemblies` dependency in that crate's `Cargo.toml` |
| call | Callee operand is a `MethodInfo` from another assembly | Build `AssemblyRef`/`TypeRef`/`MemberRef` rows in the synthetic resolution from the callee's `MethodDescription`; callee signature types translated across resolutions (primitives, object, string, non-generic non-nested classes and value types, SZ arrays) | inside the 528 lines |
| catch | `Newobj`, `Throw`, typed catch, locals, EH clause | Generator mirrors CLR `ILGenerator` (a `leave` at the end of each try/handler, `endfinally` for finally); VM maps clauses to instruction-indexed `Exception` records | inside the 528 and 250 lines |
| Regression | `reflection_dynamic_method_owner_validation_42` failed: the stub lacked the `(string, Type, Type[], Type, bool)` constructor | Added the public constructor overloads with the CLR's owner validation (null, array, interface, open generic) | ~35 lines of the 250 |

**Existing machinery that worked unchanged**, once the synthetic method existed:

- Intrinsic dispatch: `DotnetRs.MethodInfo.Invoke` and `CreateDelegate` operate on the synthetic
  method as on any `MethodInfo`; the registry hands out indexes for it normally.
- Frames and the evaluation stack, argument and local homes, and delegate invocation.
- Exception handling: the emitted `try`/`catch` and `try`/`finally` regions, `newobj`+`throw`,
  and an uncaught throw from a dynamic method surfacing to a managed `catch` in `Main` (checked in
  a throwaway probe, not a committed fixture, as is the `finally` case).
- Method and type resolution of `MemberRef`/`TypeRef` rows pointing into CoreLib and into the
  fixture assembly, including stub redirection by name.
- The VM's in-memory assembly registration (`register_owned_assembly`) needed no change.

**Not touched or not run**, so unvalidated:

- Anything that depends on CoreLib's real Emit stack: `DynamicScope`, `DynamicResolver`,
  `DynamicILGenerator`, `DynamicILInfo`, raw-IL `SetCode`, tokens from `GetTokenFor`. Libraries
  that go through `ILGenerator` abstractly work in principle; libraries that use `DynamicILInfo`,
  `ILGenerator` on `MethodBuilder`/`TypeBuilder`, or read IL bytes do not.
- Shortcuts, all honest rather than silent (each rejects with `NotSupportedException` or is listed
  here): `maxstack` is fixed at 32 instead of computed; labels and every branch opcode are
  rejected, as are field operands, `calli`, filters, faults, signatures and generic callees,
  generic or nested types in signatures, by-ref parameters and returns on callees, and any opcode
  outside a table of about 70; `owner`, module and `skipVisibility` are validated or ignored, so a
  dynamic method does not get its owner's private-member access (the VM may not enforce
  visibility anyway, which was not checked); a recorded method is never evicted, so there is no
  collectible-method lifetime (every compile adds a resolution to the metadata arena).
- Stack-trace frames for dynamic methods (name and `DeclaringType` as `null`) were not compared
  with `dotnet`; the synthetic method has a declaring type `DynamicMethods` in an assembly
  `DynamicMethod$N`, where the CLR reports none.
- The stub replaces `DynamicMethod` for every caller, including any library that constructs one
  directly. The two call-site patches for `Expression.Compile` were not removed and the host
  rungs (Newtonsoft, EF) were not rerun against it.
- `DynamicMethod.GetParameters`, `MethodHandle`, `ReturnTypeCustomAttributes`, `Define*Parameter`
  and `GetDynamicILInfo` are not implemented on the stub.
- Thread safety of concurrent compiles, GC of delegates over dynamic methods across a collection,
  and the `NEXT_DYNAMIC_ASSEMBLY` naming scheme were not tested.

**Effort and cost signal for the decision slice.** About 780 lines of new code in two files plus
three configuration lines, written in one pass. The only VM-side change is one intrinsic and a new
dependency edge from `dotnet-intrinsics-reflection` to `dotnet-assemblies` (the spike did not check
for a layering rule against it). Extending to branches and labels, fields, generic callees and
`MaxStack` is more of the same translation work. Real `DynamicILGenerator` fidelity (so
`DynamicILInfo` and IL-byte consumers work) is a separate, much larger job that this approach
sidesteps rather than solves: it would need the raw-IL decoder (an upstream dotnetdll change or
our own), which is the blocker the baseline investigation measured. The spike was written as an
experiment, not production code; it landed with this plan (see [Landing](#landing)) and plan 25
completes it.

## Conclusion (slice 5)

Decided 2026-11-15. Implement `DynamicMethod` for real, using the synthetic-module approach the
spike proved, as [plan 25](25-dynamicmethod-implementation.md). Do not implement
`AssemblyBuilder`/`TypeBuilder`.

### Answers

| Question | Answer | Where the evidence is |
|---|---|---|
| 1. Demand | Real. Newtonsoft and Microsoft.Extensions.DependencyInjection select Emit on `IsDynamicCodeCompiled` and so avoid it. System.Text.Json (on `IsDynamicCodeSupported`) and the Linq interpreter's object-array delegate for more than 2 parameters or byref types reach `new DynamicMethod`. EF Relational/SQLite reaches it through the Linq path on every query | Slice 1 findings |
| 2. Switch-level alternative | Partly. Two NativeAOT-style substitutions (`CanCompileToIL`, `CanEmitObjectArrayDelegate` false, plus a support-library thunk table) cover every Linq path up to 16 non-byref parameters and let both call-site patches go. STJ needs a per-library seam, and `DynamicMethod` throwing `PlatformNotSupportedException` is rejected because no caller catches it | Slice 3 findings |
| 3. Feasibility | Option (b), a synthetic module, with a recording front end instead of a raw-IL decoder. About 720 lines of new code, no dotnetdll change, no `MethodDescription` change. Option (a) was not attempted: it needs a new `MethodDescription` representation under every resolver cache | Slice 4 spike result |
| 4. `AssemblyBuilder`/`TypeBuilder` | Not needed; see below | This section |
| 5. Access violation | Reproduced and fixed. It was a stub-object field read (`m_handle`, `m_cache`) from CoreLib IL reached through definition-bound calls, not an invalid handle. `ldfld`/`stfld` now report a VM error naming the field and the stub | Slice 2 findings |

### Question 4: type emission

No rung in the slice 1 survey needs `TypeBuilder`. Every Emit consumer found constructs a
`DynamicMethod` and drives its `ILGenerator`: Newtonsoft's `DynamicReflectionDelegateFactory`,
DI's `ILEmitResolverBuilder`, System.Text.Json's `ReflectionEmit*MemberAccessor`, and the Linq
object-array thunk (`CreateObjectArrayDelegateRefEmit`). EF Core has no direct Emit, and the
trace of both host rungs showed 0 `DynamicMethod` constructions, 0 `LambdaCompiler.Compile`
calls and no `TypeBuilder`/`AssemblyBuilder` construction.

The one `TypeBuilder` reach in the survey is `DelegateHelpers.MakeNewCustomDelegate`
(`Expression.GetDelegateType` for byref or pointer delegate types, through `AssemblyGen`). No rung
reaches it. It fails for the same reason on the VM and on any runtime without Emit, and it is
avoidable without type emission: byref object-array thunks can be a `DynamicMethod`, and a custom
delegate type for a signature has no caller among the rungs. Type emission would also need
what `DynamicMethod` does not: a writable type table, generic type definitions, interface
implementations and method overrides in one synthetic module, and `CreateType` semantics. That is a
separate job. It is out of scope for plan 25 and restarts only if a chosen application rung reaches
`TypeBuilder` or `AssemblyBuilder.DefineDynamicAssembly` on an enabled path.

### Why `DynamicMethod`, and why now

- The demand is real and has no switch. STJ's emit-vs-reflection choice, byref Linq delegates and
  every library that emits on `IsDynamicCodeSupported` can only be served per library, and the seam
  approach stops being cheap once a fallback is not a drop-in object (slice 3).
- The spike answered the open risk. Three fixtures (`Invoke` and `CreateDelegate`, a call to an
  existing method, a throw and a typed catch) pass against `dotnet`. The existing intrinsic
  dispatch, frames, delegates, exception regions and `MemberRef` resolution worked
  unchanged on a method the loader had never seen.
- Neither blocker the baseline investigation measured (no raw-IL decoder in dotnetdll, no module
  identity) applies, because the shim records resolved operations and none of CoreLib's Emit code
  runs. The cost of the remaining work is the translation table, not a new VM concept.
- The alternative is worse. Reverting to per-library seams leaves the byref gap and every new
  library needing a patch. Option (a) hits the "Ask the maintainer" condition (a new
  `MethodDescription` representation touching every cache), and the spike showed it is unnecessary.

Verified on commit 47b30872 (maintainer session): `dynamic_method_add_42`,
`dynamic_method_call_42` and `dynamic_method_catch_42` pass `scripts/diff_run.sh`; the Newtonsoft
roundtrip and EF InMemory rungs still match `dotnet` on stdout and exit code with the stub in
place (release CLI); `./check.sh --fast`, clippy `-D warnings` and fmt were clean.

### Landing

**Decided 2026-11-15: the spike landed with plan 24.** The four `dynamic_method_*_42` fixtures
moved to `fixtures/reflection/`, and the spike labels in the stub, the intrinsic and its
`NotSupportedException` messages now name plan 25 instead. The reasons for the recommendation:

- Redoing it would repeat the same recording shim and the same one-method `Resolution` build for
  no gain; plan 25's slices extend both files in place.
- It is strictly better than main today. Any `DynamicMethod` construction on main stops at a missing
  QCall; with the stub it either runs or stops with a `NotSupportedException` naming the missing
  feature (the spike's shortcuts all reject, none is silently wrong).
- The risk is bounded and checked: the stub replaces `DynamicMethod` for every caller, and the
  maintainer session confirmed both host rungs and the existing suite stay green.
- Landing needs three things in the same change: move the three `dynamic_method_*_42` fixtures
  (and `dynamic_method_ctor_42`) from `debug_fixtures` to `fixtures/reflection/`, replace the
  `Plan 24 slice 4 SPIKE` header comments with an accurate description of the current limits, and
  keep `docs/COMPATIBILITY.md` as updated in this slice. The `dotnet-intrinsics-reflection` to
  `dotnet-assemblies` edge is acceptable (plan 25, "Layering").
- Reason to hold back instead: the spike was written in one pass and has the shortcuts listed in
  slice 4. If the maintainer prefers main to contain no code with a known gap, land only the
  fixtures, the AV fix and this decision, and let plan 25 slice 1 bring the code in already
  complete for its first scope. The cost is a second review of the same 720 lines.

## Scope

In: answering the five questions, one spike, the AV repro, and the decision.

Out: a production `DynamicMethod` implementation (now [plan 25](25-dynamicmethod-implementation.md)).
The spike was allowed to stay on a branch; the maintainer chose to land it
(see "Landing" in the Conclusion). Changing either feature value without
rerunning the host rungs is also out, because the values were chosen on rung evidence.

## Slices

1. **Demand survey.** Answer question 1, with evidence (call stacks or source references) per
   library. Note it here.
2. **AV repro.** Add the `DynamicMethod` constructor fixture (expected to differ from `dotnet`
   for now; keep it in `debug_fixtures` until it passes), then find the cause and fix it or record
   it.
3. **Switch-level experiment.** Answer question 2 on a branch with both patches removed. Rerun
   Newtonsoft, EF InMemory and the expression fixtures, and record the results.
4. **`DynamicMethod` spike.** Pick option (a) or (b) from question 3 and get a fixture that emits
   `ldarg.0; ldarg.1; add; ret` and calls it through both `Invoke` and `CreateDelegate`. Then add
   one that calls an existing method by token and one that throws and catches. Record what broke
   and how much code each step needed.
5. **Decision.** Write the conclusion in this plan, answering question 4 in it. Then either
   draft the follow-up implementation plan under the next free ID, or land the switch-level answer
   with the call-site patches removed. Update the README entries and `docs/COMPATIBILITY.md`.

## Done when

- Questions 1–5 each have a recorded answer with evidence in this plan.
- The `DynamicMethod` constructor no longer raises `AccessViolationException`, or the cause is
  recorded with a reason it can't be fixed locally.
- Either a follow-up plan exists and the README points to it, or the switch-level answer is on
  main, `call_ops_impl.rs` has no Emit call-site patches, and the host rungs still match `dotnet`.

## Ask the maintainer only if

- The demand survey finds no real-application path that needs Emit once the switch-level answer
  works. In that case confirm before dropping `DynamicMethod` entirely.
- The spike needs a dotnetdll change (a writer or body-decoder API) that must go upstream.
- Option (a) needs a new `MethodDescription` representation that touches every resolver cache.
