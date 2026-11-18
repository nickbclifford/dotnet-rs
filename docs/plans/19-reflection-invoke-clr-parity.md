# Reflection invocation matches the CLR

**Status:** Complete
**Outcome:** Every reflection-invoke fixture matches `dotnet` through `scripts/diff_run.sh` and
the diff harness; no reflection fixture encodes a deviation.

## What changed

`MethodInfo.Invoke`, both `ConstructorInfo.Invoke` overloads and `Activator.CreateInstance` run as
native effect bodies (`invoke_async` in `crates/dotnet-intrinsics-reflection/src/methods.rs`,
`activator_async` in `types/type_construction.rs`) through `run_reflection_call`. This plan
reversed two earlier decisions of [plan 08](08-async-effects.md) to match the CLR:

- **Wrapping.** Target calls go through `fx.try_call`, and a managed exception becomes a
  `TargetInvocationException` with the original as `InnerException`, unless `invokeAttr` has
  `BindingFlags.DoNotWrapExceptions` (0x02000000). Capture stops exception search at the boundary,
  so outer filters see only the wrapper. `Activator.CreateInstance<T>()` and
  `CreateInstanceDefaultCtor` use the same body. Host errors stay host errors.
- **Byref returns.** A declared byref return is read with its pointee type and boxed; a byref-like
  pointee throws `NotSupportedException`.
- **Argument binding.** Array arguments and receivers use the `castclass`/`isinst` assignability
  code. Byref arguments are copied in and copied back only when the target succeeds.
- **Overloads and strings.** `GetConstructor(Type[])` and `Activator.CreateInstance(Type, object[])`
  throw `AmbiguousMatchException`; strings are constructed through the intrinsic-constructor
  protocol.

CLR probe (.NET 10.0.14): `ref int` and `ref object` arguments are copied back only on success; a
successful call replaces the array entry, and after a throw the original box and array slots are
unchanged. A boxed `int` is rejected with `ArgumentException` for `ref long` and for `long?`, so
those widening conversions stay rejected.

## Out of scope

- `Binder`, `CallingConventions` and `ParameterModifier` arguments to `GetConstructor` are popped
  and ignored.
- `Type.InvokeMember` throws `NotSupportedException`.

Both are tracked in the plans README, "Not yet planned". `Delegate.DynamicInvoke` goes through
`MethodInfo.Invoke` in the support assembly and is diffed (`delegates/delegate_dynamic_invoke_42.cs`).

## Checks

The reflection `_42` fixtures under `crates/dotnet-cli/tests/fixtures/reflection/` (including
`invoke_wrapping_42`, `invoke_byref_return_42`, `invoke_byref_exception_42`, `invoke_nullable_42`,
`invoke_byref_42`, `invoke_byref_writethrough_42`, `activator_create_instance_args_42`,
`get_constructor_types_42` and `reflection_invoke_42`) are diff tests and pass with no features,
`multithreading`, and each with `validation-all`. `reflection_stress_0.cs` exits 0, which
`diff_test!` cannot express. `invoke_argument_race_42` races a narrowing (`long` → `int`)
conversion under a stress schedule; it does not prove a specific interleaving.
