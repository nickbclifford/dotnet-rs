# Investigate Reflection.Emit and `DynamicMethod`

**Status:** Complete
**Outcome:** A decision backed by a working spike: implement `DynamicMethod` through a synthetic
one-method module ([plan 25](25-dynamicmethod-implementation.md)); do not implement
`AssemblyBuilder`/`TypeBuilder`. The `RuntimeType.GetNativeTypeHandle` access violation is fixed.

## Answers

| Question | Answer |
|---|---|
| 1. Demand | Real. Newtonsoft.Json and Microsoft.Extensions.DependencyInjection select Emit on `IsDynamicCodeCompiled` and so avoid it. System.Text.Json selects its member accessor on `IsDynamicCodeSupported`, and the Linq interpreter's object-array delegate needs `DynamicMethod` for more than 2 parameters or byref/pointer types; EF Relational/SQLite reaches that on every query. |
| 2. Switch-level alternative | Partly. Two NativeAOT-style substitutions cover the Linq paths; STJ has no switch of its own, and EF's NativeAOT check rules out flipping `IsDynamicCodeSupported`. Making `DynamicMethod` throw `PlatformNotSupportedException` was rejected: no caller catches it. |
| 3. Feasibility | A synthetic module with a recording front end (no raw-IL decoder, no `MethodDescription` change). A resolver-backed method representation would have touched every resolver cache and was not needed. |
| 4. `AssemblyBuilder`/`TypeBuilder` | Not needed (below). |
| 5. Access violation | Reproduced and fixed (below). |

Demand survey (Newtonsoft 13.0.3, EF Core 9.0.17, DI 9.0.17, STJ and Linq from .NET 10.0.14),
with the trace evidence and probe programs, is in [evidence](evidence/24-demand-survey.txt).

## What landed

- **Linq substitutions** (`crates/dotnet-vm/src/intrinsics/linq_expressions.rs`):
  `LambdaExpression.get_CanCompileToIL` returns false, so `Compile()` uses the interpreter (the
  branch NativeAOT takes; EF does not read it), and
  `DelegateHelpers.CreateObjectArrayDelegate` runs `DotnetRs.ObjectArrayDelegates.Create`
  (`crates/dotnet-assemblies/src/support/ObjectArrayDelegates.cs`), a table of generic thunks for up
  to 16 parameters without byref, pointer or byref-like types. Other shapes use the framework's
  emitted thunk over `DynamicMethod`. These replaced two former call-site patches in
  `call_ops_impl.rs` (a `LambdaExpression.Compile()` redirect and a `ForceAllowDynamicCode` stub).
- **Access violation.** Every `System.RuntimeType` is a `DotnetRs.RuntimeType` stub whose only
  field is the type index. CoreLib IL reached through definition-bound calls read CoreLib-only
  fields (`m_handle` via `GetNativeTypeHandle`, `m_cache` via `RuntimeType.Cache`) outside the stub
  and raised `AccessViolationException`. Definition-bound calls to `System.RuntimeType` members now
  go to the stub when it implements them in IL (`crates/dotnet-assemblies/src/resolution.rs`); the
  stub implements `IsActualInterface` and `CreateInstanceImpl`; and `ldfld`/`stfld` on a stub
  report a VM error naming the field, the stub and the calling method instead of a catchable
  exception.
- **The `DynamicMethod` spike**, a recording `DotnetRs.DynamicMethod` stub that builds a one-method
  in-memory assembly. Its fixtures `dynamic_method_{add,call,catch,ctor}_42` moved to
  `fixtures/reflection/`; plan 25 extended the same files.

### Why not `TypeBuilder`

Every Emit consumer found constructs a `DynamicMethod` and drives its `ILGenerator`: Newtonsoft's
`DynamicReflectionDelegateFactory`, DI's `ILEmitResolverBuilder`, STJ's
`ReflectionEmit*MemberAccessor` and the Linq object-array thunk. The only `TypeBuilder` reach is
`DelegateHelpers.MakeNewCustomDelegate` (`Expression.GetDelegateType` for byref or pointer
delegate types), which no rung reaches. Type emission would need a writable type table, generic
type definitions, interface implementations, overrides and `CreateType` semantics. Restart only if
a chosen application rung reaches `TypeBuilder` or `AssemblyBuilder.DefineDynamicAssembly` on an
enabled path.

## Gaps found and logged elsewhere

`ParameterInfo` custom attributes, the SQLite connection-string keyword, expression trees that
capture locals (`FieldInfo.GetFieldFromHandle`), `Type.GetType(string)`/`Assembly.GetType(string)`,
and the STJ `Dictionary` alternate lookup are tracked in the plans README, "Not yet planned".

## Checks

`reflection/activator_create_instance_binding_flags_42`, `reflection/dynamic_method_owner_validation_42`
(interface, array and open-generic owners throw `ArgumentException`) and
`reflection/is_generic_parameter_42` are diff fixtures, as are the expression fixtures
`expression_compile_many_params_42` and `expression_custom_delegate_42`.
