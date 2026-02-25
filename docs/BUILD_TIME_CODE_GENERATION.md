# Build-Time Code Generation

This document describes the two build-time code generation systems that wire up instruction dispatch and intrinsic method resolution without manual registration.

## Overview

The `dotnet-vm` build script (`crates/dotnet-vm/build.rs`, ~260 lines) scans source files at compile time and generates two lookup tables:

1. **Instruction dispatch table** — maps CIL opcode discriminants to handler functions
2. **Intrinsic PHF lookup table** — maps string keys (type + method signature) to native handler functions

Both tables are generated into `$OUT_DIR/` and included via `include!()` in the compiled crate.

## Instruction Table Generation

### Source: `#[dotnet_instruction(Opcode)]` attribute

Instruction handlers are annotated in `src/instructions/**/*.rs`:

```rust
#[dotnet_instruction(Add)]
pub fn handle_add<'gc, 'm, T: VesOps<'gc, 'm> + ?Sized>(
    ctx: &mut T,
    _instr: &Instruction,
) -> StepResult { ... }
```

### Build Process

1. `process_instruction_file` walks all `.rs` files in `src/instructions/`
2. Parses each file with `syn` looking for functions with `#[dotnet_instruction(...)]`
3. Extracts the opcode variant name and the module path of the handler function
4. `generate_instruction_table` creates an array `INSTRUCTION_TABLE: [Option<InstructionHandler>; Instruction::VARIANT_COUNT]` indexed by opcode discriminant

### Runtime Usage (`dispatch/registry.rs`)

```rust
include!(concat!(env!("OUT_DIR"), "/instruction_table.rs"));

pub fn get_handler(opcode: usize) -> Option<InstructionHandler> {
    INSTRUCTION_TABLE[opcode]
}
```

`InstructionRegistry::dispatch` in `dispatch/mod.rs` calls `get_handler` to find and invoke the handler for each instruction.

## Intrinsic PHF Table Generation

### Source: `#[dotnet_intrinsic("Signature")]` and `#[dotnet_intrinsic_field("Signature")]`

Intrinsic implementations are annotated in `src/intrinsics/**/*.rs`:

```rust
#[dotnet_intrinsic("System.Math::Abs(System.Int32)")]
fn math_abs_i32(ctx: &mut dyn VesOps<'gc, 'm>, ...) -> StepResult { ... }
```

### Build Process

1. `process_intrinsic_file` walks all `.rs` files in `src/intrinsics/`
2. Parses `#[dotnet_intrinsic("...")]` attributes using `ParsedSignature` from `dotnet-macros-core`
3. Extracts: type name, member name, arity, is_static, handler path
4. Also handles `#[dotnet_intrinsic_field("...")]` for field intrinsics
5. `generate_intrinsic_phf` creates a **perfect hash function** (PHF) table via the `phf_codegen` crate

### Key Format

The intrinsic key is built from the type name, member name, arity, and static flag. The `IntrinsicRegistry::build_method_key` and `build_field_key` methods reconstruct this key at runtime to perform lookups.

Methods use the format `M:{NormalizedType}::{MemberName}#{Arity}`.
- `NormalizedType`: The canonical type name where nested type separators (`/`) are replaced with `+`.
- `Arity`: The number of parameters. If the method is an instance method, the arity includes the implicit `this` pointer (`parameters.len() + 1`).
- Example: `M:System.Math::Abs#1`

Fields use the format `F:{NormalizedType}::{MemberName}`.
- Example: `F:System.String::Empty`

When searching, `IntrinsicRegistry` formats the appropriate key string into a stack-allocated buffer (`StackWrite`) to avoid heap allocations on the hot path. The PHF table (`INTRINSIC_LOOKUP`) maps this string to a `Range` indexing into a static array `INTRINSIC_ENTRIES`.

### Runtime Usage (`intrinsics/mod.rs`)

- `IntrinsicRegistry` loads the generated PHF table
- `get()` looks up a `MethodDescription` by building a key string into a stack-allocated buffer (`StackWrite`) to avoid heap allocation on every lookup
- The resolver caches intrinsic check results (`GlobalCaches`) so repeated lookups for the same method don't rebuild the key

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

If the runtime encounters an opcode that has no registered handler, `InstructionRegistry::dispatch` returns `None`. The execution engine (`ExecutionEngine::step_normal`) will immediately panic with `panic!("Unregistered instruction: {:?}", i)`.

Similarly, for methods explicitly marked as `internal_call` in their IL metadata, if no intrinsic is found during dispatch, the engine will panic with `panic!("intrinsic not found: {:?}", method)`.

### Incremental Compilation (`cargo:rerun-if-changed`)

The build script emits the following directives:
```rust
println!("cargo:rerun-if-changed=src/instructions");
println!("cargo:rerun-if-changed=src/intrinsics");
```
These tell Cargo to skip re-running `build.rs` unless files within those directories are modified. Because parsing hundreds of source files with `syn` is computationally expensive, this ensures fast incremental builds when working on core VM components (like the GC or type system) outside the instruction and intrinsic directories.

## Generated Table Code Examples

The build process emits two main files into the `OUT_DIR`.

**1. `instruction_table.rs`**
```rust
pub const INSTRUCTION_TABLE: InstructionTable = {
    let mut table = [None; Instruction::VARIANT_COUNT];
    // Example opcode registration:
    table[88] = Some(unsafe { 
        std::mem::transmute::<*const (), crate::dispatch::registry::InstructionHandler>(
            crate::instructions::arithmetic::Add_wrapper as *const ()
        ) 
    });
    // ...
    table
};
```

**2. `intrinsics_phf.rs`**
```rust
use crate::intrinsics::static_registry::{StaticIntrinsicEntry, StaticIntrinsicHandler, Range};

pub static INTRINSIC_ENTRIES: &[StaticIntrinsicEntry] = &[
    StaticIntrinsicEntry { 
        type_name: "System.Math", 
        member_name: "Abs", 
        arity: 1, 
        is_static: true, 
        handler: StaticIntrinsicHandler::Method(unsafe { 
            std::mem::transmute::<*const (), IntrinsicHandler>(crate::intrinsics::math::math_abs_i32 as *const ()) 
        }), 
        filter: Some(crate::intrinsics::math::math_abs_i32_filter_1a2b3c4d) 
    },
    // ...
];

#[allow(dead_code)]
pub static INTRINSIC_LOOKUP: phf::Map<&'static str, Range> = ...; // PHF generated code
```

## Notes for Future Documentation

- [x] Document the exact PHF key format with examples
- [x] Explain how `IntrinsicMetadata` and `filter_name` affect dispatch
- [x] Show the generated table code (example output from a build)
- [x] Document error handling when a handler is missing for an opcode
- [x] Explain the `cargo:rerun-if-changed` directives and incremental compilation behavior
