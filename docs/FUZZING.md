# Fuzzing Infrastructure

This document describes the fuzzing infrastructure used by `dotnet-rs` to discover bugs, panics, and undefined behavior in the VM executor and value-layer primitives.

## Overview

The project uses [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (backed by libFuzzer) with the [`arbitrary`](https://docs.rs/arbitrary) crate for structured input generation. There are two independent fuzz suites:

| Suite            | Location                    | Targets                                                                           | Focus                                                              |
|------------------|-----------------------------|-----------------------------------------------------------------------------------|--------------------------------------------------------------------|
| **dotnet-vm**    | `crates/dotnet-vm/fuzz/`    | `fuzz_executor`                                                                   | CIL instruction execution, control flow, memory, exceptions        |
| **dotnet-value** | `crates/dotnet-value/fuzz/` | `fuzz_managed_ptr_offset`, `fuzz_managed_ptr_roundtrip`, `fuzz_raw_memory_access` | Pointer arithmetic, serialization roundtrips, atomic memory access |

Additionally, the `fuzzing.rs` module in `dotnet-vm` contains Miri-compatible unit tests that exercise the same `FuzzProgram` harness without requiring the full fuzzing toolchain.

## Prerequisites

```bash
# Install cargo-fuzz
cargo install cargo-fuzz

# Ensure you have a nightly Rust toolchain (required by libFuzzer)
rustup install nightly

# For the VM fuzz target, a .NET runtime must be installed
# (System.Runtime.dll is needed to resolve base types like System.Object)
# Set DOTNET_ROOT if using a non-standard install location
```

## Feature Flag: `fuzzing`

The `fuzzing` feature gate enables `Arbitrary` trait derivations on VM and value types. It propagates through the crate hierarchy:

```
dotnet-vm/fuzzing
├── dep:arbitrary
├── dotnet-value/fuzzing
│   ├── dep:arbitrary
│   ├── dotnet-types/fuzzing    (dep:arbitrary)
│   └── dotnet-utils/fuzzing    (dep:arbitrary)
├── dotnet-types/fuzzing
└── dotnet-utils/fuzzing
```

The fuzz crate Cargo.toml files (`crates/dotnet-vm/fuzz/Cargo.toml`, `crates/dotnet-value/fuzz/Cargo.toml`) depend on the respective library crates with `features = ["fuzzing"]` enabled.

> **Note:** The `fuzzing` feature is **not** included in the standard `check.sh` test matrix. It is only activated when running fuzz targets directly.

## Fuzz Targets

### `fuzz_executor` (dotnet-vm)

**Location:** `crates/dotnet-vm/fuzz/fuzz_targets/fuzz_executor.rs`

The primary fuzz target. It accepts a structured `FuzzProgram` (via `Arbitrary`) containing a sequence of `FuzzInstruction` variants, converts them into real CIL instructions via `dotnetdll`, builds a minimal .NET assembly in memory, and executes it through the VM's `Executor`.

**Key types** (defined in `crates/dotnet-vm/src/fuzzing.rs`):

- **`FuzzProgram`**: Top-level input — a list of `FuzzInstruction`s plus `num_locals` (0–10) and `num_args` (1–10).
- **`FuzzInstruction`**: An enum of ~70 CIL instruction variants covering:
  - Stack operations (`Dup`, `Pop`, `Ldnull`, `LdcI4`, `LdcI8`, `LdcR4`, `LdcR8`)
  - Arithmetic (`Add`, `Sub`, `Mul`, `Div`, `Rem`, `And`, `Or`, `Xor`, `Shl`, `Shr`, `Neg`, `Not`)
  - Indirect memory (`LdindI1`–`LdindI`, `StindI1`–`StindI`, `Cpblk`, `Initblk`, `Localloc`)
  - Control flow (`Br`, `Brtrue`, `Brfalse`, `Ret`)
  - Locals and arguments (`Ldloc`, `Stloc`, `Ldarg`, `Starg`, and address variants)
  - Object operations (`Newobj`, `Ldfld`, `Stfld`, `Ldflda`)
  - Array operations (`Newarr`, `Ldelem*`, `Stelem*`, `Ldelema`, `Ldlen`)
  - Method calls (`Call`, `Callvirt`, `Ldftn`)
  - Exception handling (`Throw`, `Rethrow`, `Leave`, `Endfinally`)

**Safety mechanisms:**
- **Instruction budget**: 10,000 instructions to prevent infinite loops.
- **Branch clamping**: Branch targets are clamped to valid instruction indices.
- **Index modulo**: Local/argument/field/method indices are taken modulo the available count.
- **Panic catching**: `panic::catch_unwind` wraps execution so panics are reported as fuzzer crashes.

**Execution environment setup:**
The harness (`execute_cil_program`) constructs a complete but minimal .NET assembly containing:
- A `FuzzType` class extending `System.Object`
- Static and instance `Int32` fields
- A constructor, the fuzz entry method `FuzzMain`, and two dummy methods
- Local variables cycling through `Int32`, `Int64`, `Float32`, `Float64`, `Object`, `String`, `IntPtr`, `Int8`, `Int16`, `UInt8`

### `fuzz_managed_ptr_offset` (dotnet-value)

**Location:** `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_offset.rs`

Fuzzes `ManagedPtr::offset()` with arbitrary pointer state and offset deltas. Validates that the resulting offset and address are computed correctly, filtering out expected overflow/underflow cases.

### `fuzz_managed_ptr_roundtrip` (dotnet-value)

**Location:** `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_roundtrip.rs`

Fuzzes the `ManagedPtr` write/read serialization roundtrip. Creates a `ManagedPtr` from arbitrary `ManagedPtrInfo`, serializes it to a byte buffer, reads it back, and asserts that origin and offset survive the roundtrip.

### `fuzz_raw_memory_access` (dotnet-value)

**Location:** `crates/dotnet-value/fuzz/fuzz_targets/fuzz_raw_memory_access.rs`

Fuzzes `AtomicAccess::store_atomic` / `load_atomic` with arbitrary offsets, sizes (1/2/4/8 bytes), values, and memory orderings. Validates that a store followed by a load returns the correctly masked value.

## Seed Corpus & Dictionary

### Seed Corpus

Pre-built seed inputs live in `crates/dotnet-vm/fuzz/corpus/fuzz_executor/` and cover:

| Seed                         | Category                           |
|------------------------------|------------------------------------|
| `seed_arithmetic_add`        | Basic arithmetic                   |
| `seed_arithmetic_div_zero`   | Division by zero                   |
| `seed_arithmetic_overflow`   | Integer overflow                   |
| `seed_control_branch`        | Branching                          |
| `seed_control_infinite_loop` | Loop with budget exhaustion        |
| `seed_memory_cpblk_null`     | Null pointer memory copy           |
| `seed_memory_localloc`       | Stack allocation + indirect access |
| `seed_memory_null_deref`     | Null dereference                   |
| `seed_stack_deep`            | Deep stack usage                   |
| `seed_stack_dup`             | Duplicate + arithmetic             |
| `seed_stack_underflow`       | Pop from empty stack               |

The `dotnet-value` targets also have pre-existing corpus directories under `crates/dotnet-value/fuzz/corpus/`.

### Corpus Generation Tools

Located in `crates/dotnet-vm/fuzz/tools/`:

- **`generate_structured_corpus.rs`** — Generates structured seed inputs using `FuzzProgram` serialized via `arbitrary`. This is the recommended corpus generator.
- **`generate_corpus.rs`** — Generates raw byte seed inputs that model CIL opcode sequences directly.

To regenerate the seed corpus:
```bash
cd crates/dotnet-vm/fuzz
cargo run -p corpus-tools --bin generate_structured_corpus --release
```

### CIL Dictionary

`crates/dotnet-vm/fuzz/cil.dict` contains common CIL opcode byte patterns to guide libFuzzer's mutation engine. Includes single-byte opcodes (`nop`, `add`, `ret`, etc.), multi-byte prefixed opcodes (`localloc`, `cpblk`, `initblk`), and common instruction sequences (`push_42`, `dup_add`, `loop_back`).

## Running the Fuzzers

### dotnet-vm executor fuzzing

```bash
cd crates/dotnet-vm

# Basic run (indefinite)
cargo +nightly fuzz run fuzz_executor

# With corpus, dictionary, and tuned options
cargo +nightly fuzz run fuzz_executor \
    fuzz/corpus/fuzz_executor \
    -- \
    -dict=fuzz/cil.dict \
    -timeout=2 \
    -rss_limit_mb=2048 \
    -max_len=10000 \
    -use_value_profile=1 \
    -print_coverage=1

# Fixed-duration run (60 seconds)
cargo +nightly fuzz run fuzz_executor \
    fuzz/corpus/fuzz_executor \
    -- -max_total_time=60

# With AddressSanitizer (recommended for finding memory bugs)
RUSTFLAGS="-Zsanitizer=address" \
    cargo +nightly fuzz run fuzz_executor \
    fuzz/corpus/fuzz_executor
```

### dotnet-value fuzzing

```bash
cd crates/dotnet-value

cargo +nightly fuzz run fuzz_managed_ptr_offset
cargo +nightly fuzz run fuzz_managed_ptr_roundtrip
cargo +nightly fuzz run fuzz_raw_memory_access
```

### Listing available targets

```bash
cargo +nightly fuzz list
```

## LibFuzzer Options

The file `crates/dotnet-vm/fuzz/fuzz_executor.options` defines default libFuzzer settings:

| Option              | Value | Purpose                                    |
|---------------------|-------|--------------------------------------------|
| `max_len`           | 10000 | Maximum input size in bytes                |
| `timeout`           | 2     | Per-test-case timeout in seconds           |
| `rss_limit_mb`      | 2048  | Memory limit per test case                 |
| `runs`              | 0     | Run indefinitely                           |
| `use_value_profile` | 1     | Enable value profiling for better coverage |
| `reduce_inputs`     | 1     | Minimize corpus to unique coverage         |
| `print_coverage`    | 1     | Report coverage periodically               |

## Corpus Management

### Minimizing

Reduce the corpus to a minimal set covering all unique code paths:

```bash
cargo +nightly fuzz cmin fuzz_executor fuzz/corpus/fuzz_executor
```

### Coverage

Inspect which code paths are being exercised:

```bash
cargo +nightly fuzz coverage fuzz_executor fuzz/corpus/fuzz_executor
```

## Crash Artifacts

When the fuzzer discovers a crash (panic, assertion failure, sanitizer violation), the triggering input is saved to:

- `crates/dotnet-vm/fuzz/artifacts/fuzz_executor/crash-<hash>`
- `crates/dotnet-value/fuzz/artifacts/<target>/crash-<hash>`

To reproduce a crash:

```bash
cargo +nightly fuzz run fuzz_executor fuzz/artifacts/fuzz_executor/crash-<hash>
```

These artifact directories are git-tracked so that known crashes serve as regression inputs.

## Miri-Compatible Unit Tests

The module `crates/dotnet-vm/src/fuzzing.rs` contains unit tests (`mod tests`) that exercise the same `execute_cil_program` harness with hand-crafted `FuzzProgram` inputs. These tests:

- Run under Miri for undefined behavior detection (using a mock `AssemblyLoader` that doesn't require a .NET runtime installation).
- Cover arithmetic, division by zero, stack underflow, locals, null throw, out-of-bounds branches, float operations, native int locals, bitwise operations, `localloc` + indirect access, and array operations.
- Provide fast CI feedback without the nightly toolchain or libFuzzer dependency.

Run them with:
```bash
cargo test --package dotnet-vm fuzzing
```

Or under Miri:
```bash
cargo +nightly miri test --package dotnet-vm fuzzing
```

## Architecture Diagram

```
┌─────────────────────────────────────────────────────┐
│                   libFuzzer                         │
│  (mutation engine, coverage feedback, corpus mgmt)  │
└──────────────────────┬──────────────────────────────┘
                       │ raw bytes
                       ▼
              ┌────────────────┐
              │   arbitrary    │
              │  (structured   │
              │  deserialization)│
              └────────┬───────┘
                       │ FuzzProgram { instructions, num_locals, num_args }
                       ▼
         ┌─────────────────────────────┐
         │  fuzzing.rs harness         │
         │  • Build Resolution         │
         │  • Convert FuzzInstruction  │
         │    → dotnetdll Instruction  │
         │  • Create Executor          │
         │  • Set instruction budget   │
         └─────────────┬───────────────┘
                       │
                       ▼
         ┌─────────────────────────────┐
         │  dotnet-vm Executor         │
         │  (full VM execution engine) │
         └─────────────────────────────┘
```

## Adding New Fuzz Targets

1. **New instruction variants**: Add to the `FuzzInstruction` enum in `crates/dotnet-vm/src/fuzzing.rs` and implement the `to_dotnetdll` conversion.
2. **New value-layer targets**: Create a new file in `crates/dotnet-value/fuzz/fuzz_targets/` and register it as a `[[bin]]` in `crates/dotnet-value/fuzz/Cargo.toml`.
3. **New seed inputs**: Either add them manually to the appropriate `corpus/` directory, or extend the corpus generation tools in `crates/dotnet-vm/fuzz/tools/`.
4. **Arbitrary support for new types**: Derive or implement `Arbitrary` behind `#[cfg(feature = "fuzzing")]` and ensure the feature propagates through `Cargo.toml`.
