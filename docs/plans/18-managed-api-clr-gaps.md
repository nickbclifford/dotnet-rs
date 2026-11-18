# Managed API gaps against the CLR

**Status:** Complete
**Outcome:** The Newtonsoft host rung matches `dotnet` again, and diff fixtures for array casts,
delegate invocation lists, `Synchronized` value-type methods, `Type`/`Object.ToString` and
`Type.IsByRefLike` match `dotnet` in ST and MT.

## What was fixed

- **One cast path.** `castclass`, `isinst` and `unbox.any`'s reference branch share
  `check_object_assignability` (`crates/dotnet-vm/src/instructions/objects/helpers.rs`), which also
  applies the ECMA-335 §I.8.7 array element equivalences (`State[]` ↔ `int[]`, `int[]` ↔
  `uint[]`). `unbox.any` to an array type used to fail, breaking Newtonsoft in
  `SZArrayHelper<T>.CopyTo`.
- **Delegates.** `Delegate.GetInvocationList` is virtual on the support `Delegate`, overridden by
  `MulticastDelegate`.
- **`Synchronized` on value types.** The CLR rejects a value type with a synchronized method,
  including a static one, with `TypeLoadException` ("Synchronized Method in Value Type."); the VM
  matches ([CLR probe](evidence/18-synchronized-value-type-clr.txt)).
- **Type names.** One CLR name-formatting helper (generic arguments, nested `+`, arrays, pointers,
  byrefs) serves `Type.ToString`, `FullName` and `Object.ToString` for boxed values and arrays.
- **`Type.IsByRefLike`** uses `invoke_args::is_byref_like`.
- **Dynamic code switch.** Newtonsoft uses Reflection.Emit when `IsDynamicCodeCompiled` is true,
  and EF Core treats `IsDynamicCodeSupported == false` as NativeAOT. An intrinsic
  (`crates/dotnet-vm/src/intrinsics/app_context.rs`) reports `IsDynamicCodeCompiled` false while
  `IsDynamicCodeSupported` stays true, which is what an interpreting runtime reports
  ([host-rung evidence](evidence/18-array-casts-host-rungs.txt)). Expression-tree compilation is now
  handled by the `LambdaExpression` intrinsics of [plan 24](24-reflection-emit-investigation.md).
- `new string(char[], int, int)` is implemented.

## Out of scope

`stelem` array-store checks (`ArrayTypeMismatchException`) are not implemented; tracked in the
plans README, "Not yet planned".

## Checks

Diff fixtures: `arrays/jagged_interface_cast_42.cs`, `delegate_invocation_list_42.cs`,
`threading/synchronized_method_42.cs`, `type_names_42.cs`, `is_byref_like_42.cs` and
`strings/ctor_char_array_range_42.cs`, in ST and MT, with and without `validation-all`.
`basic/runtime_feature_dynamic_code_42.cs` pins the dynamic-code switch (not diffed: the CLR reports
`IsDynamicCodeCompiled` true). Rerun the Newtonsoft and EF host rungs with
`scripts/diff_run.sh --host`.
