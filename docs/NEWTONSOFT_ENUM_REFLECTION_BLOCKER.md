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
