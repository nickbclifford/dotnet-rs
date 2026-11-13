# Managed API gaps against the CLR

**Status:** Complete
**Outcome:** The Newtonsoft host rung matches `dotnet` again, and new diff fixtures for array
casts, delegate invocation lists, `Synchronized` value-type methods, `Type`/`Object.ToString` and
`Type.IsByRefLike` match `dotnet` in ST and MT.

## Problem

- **`unbox.any` rejects arrays.** The Newtonsoft roundtrip
  (`crates/dotnet-cli/tests/host_projects/newtonsoft_roundtrip`) fails at IP 36 of
  `SZArrayHelper<T>.CopyTo` (`crates/dotnet-assemblies/src/support/Array.cs`), which is
  `unbox.any !T` with `T = State[]` (IP is the instruction index). `unbox_any`
  (`crates/dotnet-vm/src/instructions/objects/boxing.rs`) takes its reference branch for a
  vector target but checks only the generic collection interfaces, then falls back to
  `is_a(System.Array, State[])`, which is false. `castclass`/`isinst` use
  `check_object_assignability` (`instructions/objects/casting.rs`), which handles vector targets.
  Likely regression: `82902cf3` (2026-07-02) made `isinst` accept a vector as `ICollection<T>`, so
  `List<T>(IEnumerable<T>)` began taking the `CopyTo` path, without the matching vector check in
  `unbox.any`. The rung passed at `8fc4f2c5` (2026-06-30).
- **Array element compatibility.** `vector_matches_array_type` requires exact element equality
  for value types, so the ECMA-335 §I.8.7 equivalences (`State[]` ↔ `int[]`, `int[]` ↔ `uint[]`)
  are rejected by every cast.
- **Delegates.** The `System.Delegate` stub (`support/Delegate.cs`) has no `GetInvocationList`;
  only `MulticastDelegate` does. `delegate is MulticastDelegate` may be false for CoreLib
  delegates (`Func<>`, `Action`) because `comparer.is_subclass_of` walks the real CoreLib parent
  without the stub remap that `ancestors()` applies; `dbf34e7e` may already have fixed this.
- **`Synchronized` on value types.** The CLR rejects value types with synchronized methods at
  type load, including static methods (see `evidence/18-synchronized-value-type-clr.txt`).
- **`ToString`.** The `RuntimeType` stub (`support/RuntimeType.cs`) has no `ToString` override,
  so CoreLib's `Type.ToString` returns `"Type: " + Name`; `ValueType.ToString` inherits this.
  `FullName` omits generic arguments and nested `+`. `Object.ToString`
  (`crates/dotnet-vm/src/intrinsics/object_ops.rs`) returns `"System.ValueType"` for every boxed
  value and `"System.Array"` for every array.
- **`Type.IsByRefLike`** (`crates/dotnet-intrinsics-reflection/src/types/type_queries.rs`) always
  pushes 0, although `invoke_args::is_byref_like` exists in the same crate.

## Scope

In: the items above. Out: reflection invocation (plan 19), GC and finalizers (plan 20), and
`stelem` array-store checks (`ArrayTypeMismatchException`), which no known program depends on;
add a README entry if a fixture shows otherwise.

## Design

- One cast path: move `check_object_assignability` to `instructions/objects/helpers.rs` and use
  it from `castclass`, `isinst` and `unbox.any`'s reference branch. Add §I.8.7 element
  compatibility there, not per instruction.
- CLR type-name formatting (generic arguments, nested `+`, arrays, pointers, byrefs) in one Rust
  helper used by `Type.ToString`, `FullName` and `Object.ToString`, next to
  `RuntimeType::get_name`.
- Each slice starts with a CLR probe for any behavior not stated in the ECMA spec or CoreLib.

## Slices

1. **Array casts and Newtonsoft.** Shared cast path and §I.8.7 element compatibility. Fixture
   `arrays/jagged_interface_cast_42.cs`: `int[][].ToList()`, `new List<State[]>(jagged)`,
   `(IList<int[]>)(object)jagged`, generic `(T)(object)arr` with `T = int[]`, `foreach` over
   `IEnumerable<int[]>`, `(int[])(object)stateArray` and `(uint[])(object)intArray`. Register it
   in the diff harness; rerun the Newtonsoft and EF host rungs with `scripts/diff_run.sh --host`
   and record the result in `docs/plans/evidence/`.
2. **Delegates.** Virtual `GetInvocationList` on `Delegate`, overridden by `MulticastDelegate`;
   fold `InvocationEntries` into it if nothing else uses it. Fixture: `GetInvocationList` through
   a `Delegate`-typed reference for single and combined delegates, and `is MulticastDelegate` /
   `is Delegate` for user and CoreLib delegate types. Fix the comparer walk only if the fixture
   fails.
3. **`Synchronized` value-type methods.** Probe the CLR (type-load rejection, lock on a box, or
   no lock), then match it. Extend `threading/synchronized_method_42.cs` or add a fixture.
4. **Type names.** CLR name formatting helper; `RuntimeType.ToString`, `FullName`, and boxed and
   array `Object.ToString`. Fixture: `$"{x.GetType()}"` and `ToString()` for primitives, enums,
   structs, nested and generic types, arrays (including jagged and multidimensional), and a boxed
   struct without an override.
5. **`IsByRefLike`.** Use `is_byref_like` for `Type`/`Generic` runtime types, false otherwise;
   confirm `TypedReference`. Fixture covers `Span<T>`, `ReadOnlySpan<T>`, a user `ref struct`,
   an ordinary struct, a class and `TypedReference`.
6. **Docs.** Remove the README entries this plan closes (Newtonsoft array copying, and the
   delegate, `Synchronized`, `ToString` and `IsByRefLike` items under "Other missing or incorrect
   runtime behavior"); update `COMPATIBILITY.md` once Newtonsoft passes.
7. **Newtonsoft after slice 1.** Added after slice 1, when Newtonsoft failed with an
   `AccessViolationException` in `RuntimeType.GetNativeTypeHandle` during `DynamicMethod.Init`
   ([evidence](evidence/18-array-casts-host-rungs.txt)).
   - Cause: Newtonsoft 13.0.3 (net6.0) uses Reflection.Emit when
     `RuntimeFeature.IsDynamicCodeCompiled` is true. CoreLib returns `IsDynamicCodeSupported`,
     which `82902cf3` (2026-07-02) let default to true. Seeding the switch to false again (as
     `a527aa54` had) was tried and rejected: EF Core treats it as NativeAOT and throws "Model
     building is not supported when publishing with NativeAOT".
   - Fix: an intrinsic makes `IsDynamicCodeCompiled` false while `IsDynamicCodeSupported` stays
     true. That is the documented meaning for a runtime that interprets (Mono's interpreter
     reports the same), and it moves Newtonsoft and DI to their reflection paths.
     `basic/runtime_feature_dynamic_code_42.cs` pins it (not diffed; the CLR reports true).
   - Deserialization then reached `new string(char[], int, int)`, which had no intrinsic and
     failed with `Invalid instruction pointer: 0`. It is now implemented, with
     `strings/ctor_char_array_range_42.cs` as a diff test. This was never a regression: the
     roundtrip project (added 2026-11-11) is the first rung that deserializes; the June rung only
     serialized.
   - Not changed: because `IsDynamicCodeSupported` stays true, the `LambdaExpression.Compile()`
     redirect and the `ForceAllowDynamicCode` stub in `call_ops_impl.rs` remain (README entry
     "Dynamic code without Reflection.Emit").

## Done when

- The Newtonsoft host rung matches `dotnet` in ST and MT, and the EF rung still does.
- Every new fixture is a diff test and passes in ST and MT, with and without `validation-all`.
- `castclass`, `isinst` and `unbox.any` share one assignability function.

## Ask the maintainer only if

- The CLR rejects `Synchronized` value-type methods at type load and matching that needs
  load-time validation the VM does not have.
- Shared casting changes behavior in an existing fixture that the CLR agrees with.
- Newtonsoft fails further along after slice 1 with an unrelated gap.
