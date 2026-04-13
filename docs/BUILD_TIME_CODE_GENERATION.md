# Build-Time Code Generation

This document describes the two build-time code generation systems that wire up instruction dispatch and intrinsic method resolution without manual registration.

## Overview

The `dotnet-vm` build script (`crates/dotnet-vm/build.rs`) scans source files at compile time and generates two lookup tables:

1. **Instruction dispatch** — generates a monomorphic `match`-based dispatcher for CIL instructions
2. **Intrinsic PHF lookup table** — maps string keys to native handler IDs and generates ID-based dispatchers

Both tables are generated into `$OUT_DIR/` and included via `include!()` in the compiled crate.

## Instruction Table Generation

### Source: `#[dotnet_instruction(Opcode)]` attribute

Instruction handlers are annotated in `src/instructions/**/*.rs`:

```rust
#[dotnet_instruction(Add)]
pub fn handle_add<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    _instr: &Instruction,
) -> StepResult { ... }
```

### Build Process

1. `process_instruction_file` walks all configured instruction roots. By default this is `src/instructions/`, plus optional extra roots from `DOTNET_VM_EXTRA_INSTRUCTION_SOURCES`.
2. Parses each file with `syn` looking for functions with `#[dotnet_instruction(...)]`
3. Extracts the opcode variant name and the module path of the handler function
4. `generate_instruction_table` creates `instruction_dispatch.rs` containing the `dispatch_monomorphic` function.

### Runtime Usage (`dispatch/registry.rs`)

```rust
include!(concat!(env!("OUT_DIR"), "/instruction_dispatch.rs"));

pub fn dispatch<'gc, T: VesOps<'gc>>(
    ctx: &mut T,
    instr: &Instruction,
) -> StepResult {
    dispatch_monomorphic(ctx, instr)
}
```

`InstructionRegistry::dispatch` in `dispatch/mod.rs` calls this function to execute each instruction. The monomorphic dispatcher uses a `match` on the `Instruction` enum, allowing the Rust compiler to inline and optimize instruction handlers effectively.

## Intrinsic PHF Table Generation

### Source: `#[dotnet_intrinsic("Signature")]` and `#[dotnet_intrinsic_field("Signature")]`

Intrinsic implementations are annotated in VM-local and extracted intrinsic crates:
- `src/intrinsics/**/*.rs` (`dotnet-vm` local intrinsic registry/metadata helpers)
- `../dotnet-intrinsics-*/src/**/*.rs` (handler crates)

```rust
#[dotnet_intrinsic("System.Math::Abs(System.Int32)")]
fn math_abs_i32<T: VesOps<'gc>>(ctx: &mut T, ...) -> StepResult { ... }
```

### Build Process

1. `process_intrinsic_file` walks all configured intrinsic roots:
   - `src/intrinsics`
   - `../dotnet-intrinsics-core/src`
   - `../dotnet-intrinsics-delegates/src`
   - `../dotnet-intrinsics-span/src`
   - `../dotnet-intrinsics-string/src`
   - `../dotnet-intrinsics-threading/src`
   - `../dotnet-intrinsics-reflection/src`
   - `../dotnet-intrinsics-unsafe/src`
   - Optional extras from `DOTNET_VM_EXTRA_INTRINSIC_SOURCES`
2. Parses `#[dotnet_intrinsic("...")]` attributes using `ParsedSignature` from `dotnet-macros-core`
3. Extracts: type name, member name, arity, is_static, handler path
4. Also handles `#[dotnet_intrinsic_field("...")]` for field intrinsics
5. `generate_intrinsic_phf` creates:
    - `intrinsics_dispatch.rs`: Contains `MethodIntrinsicId` and `FieldIntrinsicId` enums and their respective `dispatch_*` functions.
    - `intrinsics_phf.rs`: Contains the perfect hash function (PHF) table via the `phf_codegen` crate.

### Key Format

The intrinsic key is built from the type name, member name, arity, and static flag. The `IntrinsicRegistry::build_method_key` and `build_field_key` methods reconstruct this key at runtime to perform lookups.

Methods use the format `M:{NormalizedType}::{MemberName}#{Arity}:{StaticFlag}`.
- `NormalizedType`: The canonical type name where nested type separators (`/`) are replaced with `+`.
- `Arity`: The number of parameters. If the method is an instance method, the arity includes the implicit `this` pointer (`parameters.len() + 1`).
- `StaticFlag`: `S` for static methods, `I` for instance methods.
- Example: `M:System.Math::Abs#1:S`

Fields use the format `F:{NormalizedType}::{MemberName}:{StaticFlag}`.
- Example: `F:System.String::Empty:S`

When searching, `IntrinsicRegistry` formats the appropriate key string into a stack-allocated buffer (`StackWrite`) to avoid heap allocations on the hot path. The PHF table (`INTRINSIC_LOOKUP`) maps this string to a `Range` indexing into a static array `INTRINSIC_ENTRIES`. Each entry in `INTRINSIC_ENTRIES` stores a handler ID (`MethodIntrinsicId` or `FieldIntrinsicId`) instead of a function pointer.

### Runtime Usage (`intrinsics/mod.rs`)

- `IntrinsicRegistry` loads the generated PHF table and ID-based dispatchers.
- `get()` looks up a `MethodDescription` by building a key string and retrieving a range of candidates from the PHF.
- Each candidate is checked against its filter function.
- The selected candidate's `MethodIntrinsicId` is returned.
- Dispatch is performed via `dispatch_method_intrinsic`, which matches on the ID.

### Metadata and Filtering

- `IntrinsicEntry` supports a `filter_name` for conditional intrinsic behavior. When parsing a `#[dotnet_intrinsic]` attribute, the build script automatically generates a filter function name based on the handler name and a hash of the signature.
- This filter is stored in the `StaticIntrinsicEntry`. At runtime, `IntrinsicRegistry::get_metadata` invokes the filter function with the current `MethodDescription`. The handler is only selected if the filter returns `true` (or if no filter is present).
- This allows multiple intrinsic handlers to register for the same string key (e.g., generic methods where the behavior depends on the generic type arguments), using the filter to disambiguate at resolution time.
- `get_metadata()` provides an `IntrinsicMetadata` struct that includes the `IntrinsicKind` (`Static` or `VirtualOverride`), the underlying handler, and the filter function. This information helps the runtime determine if an intrinsic should bypass normal virtual dispatch.

## Proc Macro Crates

### `dotnet-macros` (`crates/dotnet-macros/src/lib.rs`)
- Defines the proc-macro attributes: `#[dotnet_instruction]`, `#[dotnet_intrinsic]`, `#[dotnet_intrinsic_field]`
- Thin wrapper delegating to `dotnet-macros-core`

### `dotnet-macros-core` (`crates/dotnet-macros-core/src/lib.rs`)
- Shared parsing logic used by both the proc macros and the build script
- `ParsedSignature`: Parses `.NET` method signatures (namespace, type, method, parameters)
- `ParsedFieldSignature`: Parses field signatures

## Non-Obvious Connections

### Build Script ↔ Proc Macros Share Code
The build script and the proc macros both depend on `dotnet-macros-core` for signature parsing. This ensures the key format is consistent between compile-time table generation and runtime lookups.

### Build Script ↔ `dotnetdll`
The instruction table generation depends on `dotnetdll::prelude::Instruction::VARIANT_COUNT` to size the array, tying the dispatch table to the exact set of CIL opcodes the metadata parser knows about.

### Hash-Based Deduplication
The build script uses `DefaultHasher` to deduplicate handler registrations — if two files somehow register the same opcode, the build detects this.

### Missing Handler Error Behavior

If the runtime encounters an opcode that has no registered handler, the generated `dispatch_monomorphic` function returns a `VmError::Execution(ExecutionError::NotImplemented)`.

Similarly, for methods explicitly marked as `internal_call` in their IL metadata, if no intrinsic is found during dispatch, the engine returns `StepResult::Error(VmError::Execution(ExecutionError::NotImplemented(...)))`.

### Incremental Compilation (`cargo:rerun-if-changed`)

The build script emits stable and dynamic directives:
```rust
println!("cargo:rerun-if-changed=src/instructions");
println!("cargo:rerun-if-changed=src/intrinsics");
println!("cargo:rerun-if-env-changed=DOTNET_VM_EXTRA_INSTRUCTION_SOURCES");
println!("cargo:rerun-if-env-changed=DOTNET_VM_EXTRA_INTRINSIC_SOURCES");
// and one cargo:rerun-if-changed=<root> per configured source root
```
These tell Cargo to skip re-running `build.rs` unless files within those directories are modified. Because parsing hundreds of source files with `syn` is computationally expensive, this ensures fast incremental builds when working on core VM components (like the GC or type system) outside the instruction and intrinsic directories.

## Generated Table Code Examples

The build process emits two main files into the `OUT_DIR`.

**1. `instruction_dispatch.rs`**
```rust
pub fn dispatch_monomorphic<'gc, T: crate::stack::ops::VesOps<'gc>>(
    ctx: &mut T,
    instr: &Instruction,
) -> crate::StepResult {
    match instr {
        Instruction::Add => crate::instructions::arithmetic::handle_add(ctx, instr),
        // ...
        _ => crate::StepResult::Error(...)
    }
}
```

**2. `intrinsics_dispatch.rs`**
```rust
pub enum MethodIntrinsicId {
    Missing,
    System_Math_Abs_1a2b3c4d,
    // ...
}

pub fn dispatch_method_intrinsic<'gc, T: VesOps<'gc>>(
    id: MethodIntrinsicId,
    ctx: &mut T,
    // ...
) -> StepResult {
    match id {
        MethodIntrinsicId::System_Math_Abs_1a2b3c4d => dotnet_intrinsics_core::math::math_abs_i32(ctx, ...),
        // ...
    }
}
```

**3. `intrinsics_phf.rs`**
```rust
use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Range};

pub static INTRINSIC_ENTRIES: &[StaticIntrinsicEntry] = &[
    StaticIntrinsicEntry { 
        type_name: "System.Math", 
        member_name: "Abs", 
        arity: 1, 
        is_static: true, 
        handler: StaticIntrinsicHandler::Method(MethodIntrinsicId::System_Math_Abs_1a2b3c4d), 
        filter: Some(crate::intrinsics::math::math_abs_i32_filter_1a2b3c4d) 
    },
    // ...
];

pub static INTRINSIC_LOOKUP: phf::Map<&'static str, Range> = ...;
```

## dotnet-cli Fixture Test Generation (`crates/dotnet-cli/build.rs`)

`dotnet-cli` also uses build-time generation for integration test wiring. Its `build.rs` scans `tests/fixtures/**/*.cs` and emits Rust test macros into `$OUT_DIR/tests.rs`.

Generated forms:

- `fixture_test!(name, dll_path, expected_exit_code)` for regular fixture runs.
- `multi_arena_test!(name, fixture_rel_path, thread_count, expected_exit_code)` for inherently concurrent fixtures that require multiple executor arenas.

The build script applies feature-aware ignore policy during generation:

- `monitor_try_enter_timeout_42` and `circular_init_mt_42` are ignored outside `multithreading`.
- `generic_constraints_fail_0` is ignored unless `generic-constraint-validation` is enabled.

Fixture binaries are built under Cargo's target tree (`DOTNET_FIXTURES_BASE`) and a fixture content hash is used to skip unnecessary rebuilds.

## Notes for Future Documentation

- [x] Document the exact PHF key format with examples
- [x] Explain how `IntrinsicMetadata` and `filter_name` affect dispatch
- [x] Show the generated table code (example output from a build)
- [x] Document error handling when a handler is missing for an opcode
- [x] Explain the `cargo:rerun-if-changed` directives and incremental compilation behavior
