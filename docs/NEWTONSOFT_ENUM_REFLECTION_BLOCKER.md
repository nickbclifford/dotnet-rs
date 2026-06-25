# Newtonsoft enum reflection blocker (Step 5.6 investigation)

Date: 2026-06-24

## Scope

Investigate why Newtonsoft serialization fails after the 5.4 hang fix with:

- `System.NotSupportedException: NotSupported_AbstractNonCLS`
- top frame: `System.Reflection.FieldInfo.GetRawConstantValue()`
- call path: `System.Type.GetEnumData` -> `System.Enum.GetNames` -> `Newtonsoft.Json.Utilities.EnumUtils.InitializeValuesAndNames`

No product-code fix is included in this step.

## Reproduction evidence

### 1) Rung-2 probe still fails in host mode

```bash
bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj
```

Observed (dotnet-rs side):

- `Unhandled Exception: System.NotSupportedException: NotSupported_AbstractNonCLS`
- stack includes `System.Reflection.FieldInfo.GetRawConstantValue()` and `System.Type.GetEnumData(...)`

### 2) Minimal enum probe isolates the reflection behavior

A tiny app that calls `typeof(MyEnum).GetFields(...)` and then `field.GetRawConstantValue()` prints:

- stock dotnet: field runtime type is `System.Reflection.MdFieldInfo`, raw values returned
- dotnet-rs: field runtime type is `DotnetRs.FieldInfo`, `GetRawConstantValue()` throws `NotSupported_AbstractNonCLS`

This proves dotnet-rs is surfacing custom field-info objects and those objects do not provide the required override.

## Root cause

This is **not** a virtual-dispatch engine miss. It is a reflection runtime-type implementation gap.

1. `System.Type.GetEnumData` (CoreLib source) iterates enum fields and calls:
   - `flds[i].Name`
   - `flds[i].GetRawConstantValue()`
2. dotnet-rs reflection object creation for fields (`crates/dotnet-intrinsics-reflection/src/common.rs`) always creates `DotnetRs.FieldInfo` instances.
3. `DotnetRs.FieldInfo` (`crates/dotnet-assemblies/src/support/FieldInfo.cs`) does **not** override `GetRawConstantValue()`.
4. The reflection field intrinsic table (`crates/dotnet-intrinsics-reflection/src/fields.rs`) has handlers for `GetName`, `GetDeclaringType`, and `GetFieldHandle`, but no `GetRawConstantValue` handler.
5. Therefore the virtual call lands on `System.Reflection.FieldInfo` base implementation, which throws `NotSupported_AbstractNonCLS`.

Why this is not a dispatch bug:

- Calls like `field.Name` on the same `DotnetRs.FieldInfo` instances do dispatch to dotnet-rs overrides/intrinsics correctly.
- Failure only occurs for a member that has no override on `DotnetRs.FieldInfo`.

## Fix plan (future step, not implemented here)

1. **Support type override:**
   - Add `GetRawConstantValue` override to `DotnetRs.FieldInfo` in `support/FieldInfo.cs` as an internal-call surface.
2. **Intrinsic implementation:**
   - Add `DotnetRs.FieldInfo::GetRawConstantValue()` handler in `dotnet-intrinsics-reflection/src/fields.rs`.
   - Resolve runtime field descriptor from reflection object index.
   - Read constant metadata from field definition (`field.default` / `field.literal`).
   - Return boxed managed object for the constant value (enum path requires numeric constants at minimum).
   - Match CoreLib behavior for non-constant fields (throw `InvalidOperationException`).
3. **Regression coverage:**
   - Add a focused reflection regression test exercising `Enum.GetNames` / `FieldInfo.GetRawConstantValue`.
   - Re-run rung-2 parity: `bash scripts/diff_run.sh /tmp/nuget-probe/App.csproj`.

## Additional adjacent gap discovered (not fixed in this step)

`DotnetRs.FieldInfo.GetFieldAttributes()` currently has no implementation path (extern internal-call without matching intrinsic), causing:

- `Internal VM error: Method execution failed: Not implemented: no body in executing method: DotnetRs.FieldInfo.GetFieldAttributes`

This is separate from the `GetRawConstantValue` blocker but should be tracked as follow-up reflection completeness work.

## Post-5.7 follow-on blocker (not fixed in this step)

After implementing `DotnetRs.FieldInfo.GetRawConstantValue()` support and intrinsic constant materialization, rung-2 no longer fails with `NotSupported_AbstractNonCLS`; execution progresses further and now fails with:

- `System.InvalidProgramException: Common Language Runtime detected an invalid program.`
- top frame: `System.Globalization.CompareInfo/SortHandleCache.GetCachedSortHandle(string sortName)`
- call path still rooted under enum metadata initialization (`System.Type.GetEnumData` -> `System.Enum.GetNames` -> `Newtonsoft.Json.Utilities.EnumUtils.InitializeValuesAndNames`)

This is a newly exposed downstream runtime bug and is outside the scope of the 5.6/5.7 field-constant reflection fix.

## Post-5.8 enum coercion fix (step 5.9)

Date: 2026-06-24

### Root cause

The `InvalidProgramException` in `SortHandleCache.GetCachedSortHandle` (and related enum comparison paths) is caused by enum values landing on the eval stack as `StackValue::ValueType` — e.g. when read from a generic value-type slot such as a `Dictionary<TKey, TEnum>` — rather than as their underlying primitive (e.g. `Int32`). ECMA-335 §III.1.1.1 requires the CLI to treat enum types exactly as their underlying type for all numeric opcodes; our VM was not normalizing these before arithmetic, comparison, branch, and conversion operations, triggering `InvalidProgramException` when the opcode's type-check failed.

### Fixes applied (step 5.9)

**Core fix — `StackValue::coerce_enum_to_underlying()`** (`crates/dotnet-value/src/stack_value.rs`):
- New `enum_value_type_to_underlying()` helper reads the `value__` field (offset 0) from a boxed/struct enum and returns the underlying primitive, widening sub-`int` types to `Int32` per stack normalization.
- New public `coerce_enum_to_underlying()` method on `StackValue` — a no-op for non-enum values, applied before every affected opcode.

**Opcode sites patched** (all pop-and-operate opcodes):
- All conditional branch instructions (`beq`, `bge`, `bgt`, `ble`, `blt`, `bne`, `brtrue`, `brfalse`, `switch`) — `crates/dotnet-vm/src/instructions/flow.rs`
- All binary arithmetic / comparison / shift macros (`binary_op!`, `binary_op_result!`, `binary_op_sgn!`, `comparison_op!`, `unary_op!`) — `crates/dotnet-vm/src/instructions/macros.rs`
- Numeric conversion (`conv`) — `crates/dotnet-vm/src/instructions/conversions.rs`

**Additional fixes in this batch:**
- `String.Equals(string, string, StringComparison)` intrinsic — `crates/dotnet-intrinsics-string/src/operations.rs` — the `SortHandleCache` path calls this with a `StringComparison` enum argument; maps all comparison modes to ordinal (locale collation not yet implemented).
- `unbox.any` fast-path for already-unboxed primitives — `crates/dotnet-vm/src/instructions/objects/boxing.rs` — `Array.GetValue` returns primitives directly; `unbox.any` now accepts these rather than throwing `InvalidProgramException`.
- `ldfld` intercept for `System.String._stringLength` / `_firstChar` — `crates/dotnet-vm/src/instructions/objects/fields.rs` — CoreLib reads these internal fields directly in `String.GetHashCode` and related paths; strings are stored as `HeapStorage::Str`, not field-laid-out objects, so these are intercepted here.
- PInvoke enum return unwrapping — `crates/dotnet-pinvoke/src/call.rs` — enums returned from native code are now represented on the eval stack as their underlying primitive rather than as a `ValueType`.

**Reflection completeness fixes (step 5.8 + adjacent)** (`crates/dotnet-intrinsics-reflection/`):
- `DotnetRs.FieldInfo::GetFieldAttributes()` — reconstructs `FieldAttributes` flags from metadata bits.
- `DotnetRs.FieldInfo::GetValue(object)` — returns boxed constant for literal (enum member) fields; returns null for static non-literal fields; throws `NotImplementedException` for instance non-literal fields.
- `DotnetRs.FieldInfo::GetCustomAttributes(bool)` / `GetCustomAttributes(Type, bool)` / `IsDefined(Type, bool)`.
- `DotnetRs.MethodInfo::IsDefined(Type, bool)` / `DotnetRs.ConstructorInfo::IsDefined(Type, bool)`.
- `MethodInfo::get_ContainsGenericParameters` — always returns false (all dispatched methods are fully resolved).
- `System.Type::GetEnumUnderlyingType()` / `System.RuntimeType::GetEnumUnderlyingType()`.
- `System.Type::get_IsGenericTypeDefinition()` / `System.RuntimeType::get_IsGenericTypeDefinition()`.
- `DotnetRs.ParameterInfo::GetMember()` — backed by new internal-call intrinsic; `ParameterInfo.Member` property now delegates to it.
- Generic type definition representation: `make_runtime_type` and `runtime_type_from_concrete` now produce `RuntimeType::Generic` with open `TypeParameter` args for uninstantiated generic type definitions, so `IsGenericTypeDefinition` can identify them correctly.
- `dotnet-macros-core`: intrinsic signature parser now handles `valuetype`/`class` IL-style type-kind qualifiers (needed for `FieldAttributes` return type annotation).
- Improved panic message for unimplemented method-info intrinsics (includes type name, method name, param count).
