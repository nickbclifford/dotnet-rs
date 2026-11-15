# Investigate Reflection.Emit and `DynamicMethod`

**Status:** Slices 1 (demand survey) and 2 (AV) done; slices 3–5 not started
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
`debug_fixtures/dynamic_method_ctor_42` stays out of the suite until `DynamicMethod` works.

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

## Scope

In: answering the five questions, one spike, the AV repro, and the decision.

Out: a production `DynamicMethod` implementation. The spike may stay on a branch, and only the
fixtures, the AV fix and the decision record land on main. Changing either feature value without
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
