# Fuzzing Infrastructure

This document describes the fuzzing infrastructure used by `dotnet-rs` to discover bugs, panics, and undefined behavior in the VM executor and value-layer primitives.

## Overview

The project uses [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (backed by libFuzzer) with the [`arbitrary`](https://docs.rs/arbitrary) crate for structured input generation. There are two independent fuzz suites:

| Suite            | Location                    | Targets                                                                           | Focus                                                              |
|------------------|-----------------------------|-----------------------------------------------------------------------------------|--------------------------------------------------------------------|
| **dotnet-vm**    | `crates/dotnet-vm/fuzz/`    | `fuzz_executor`                                                                   | CIL instruction execution, control flow, memory, exceptions        |
| **dotnet-value** | `crates/dotnet-value/fuzz/` | `fuzz_managed_ptr_offset`, `fuzz_managed_ptr_roundtrip`, `fuzz_raw_memory_access`, `fuzz_managed_ptr_rebase` | Pointer arithmetic, serialization roundtrips, atomic memory access |

Additionally, the `fuzzing.rs` module in `dotnet-vm` contains unit tests that exercise the same `FuzzProgram` harness without requiring the fuzzing toolchain (see [Harness unit tests](#harness-unit-tests)).

## Where fuzzing runs

- `fuzz-corpus-replay` in `ci.yml` is blocking. It replays the committed corpora for the targets
  `select-deep` chooses from the changed paths ([CI.md](CI.md#test-hierarchy-and-migration)), and
  all of them in full sweeps.
- `fuzz.yml` is advisory. It runs duration-based campaigns and crash minimization on a daily
  schedule and on manual dispatch. A green run does not show that every campaign completed; read
  the step logs. Triage saved crashes and promote useful ones to a corpus or unit test.

## Prerequisites

The commands below use `$NIGHTLY`, the pinned nightly set in
[CI.md](CI.md#toolchains-and-miri-flags). CI builds the `cargo-fuzz` driver with the stable
toolchain from `rust-toolchain.toml`:

```bash
cargo install cargo-fuzz --version 0.13.1 --locked
rustup toolchain install "$NIGHTLY"

# For the VM fuzz target, a .NET runtime must be installed
# (System.Runtime.dll is needed to resolve base types like System.Object)
# Set DOTNET_ROOT if using a non-standard install location
```

## Feature Flag: `fuzzing`

The `fuzzing` feature gate enables `Arbitrary` trait derivations on VM and value types. It propagates through the crate hierarchy:

> **Newtype carve-out:** `Arbitrary` derives for `dotnet-utils` newtypes deliberately construct
> unvalidated values. The derives expand in `newtypes.rs` and may access private tuple fields,
> while external callers cannot, preserving the privatization boundary.

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

The shared feature matrix (used by CI and `./check.sh --full`) has a `fuzzing` leg;
`./check.sh --fast` does not.

## Fuzz Targets

### `fuzz_executor` (dotnet-vm)

**Location:** `crates/dotnet-vm/fuzz/fuzz_targets/fuzz_executor.rs`

The primary fuzz target. It accepts a structured `FuzzProgram` (via `Arbitrary`) containing a sequence of `FuzzInstruction` variants, converts them into real CIL instructions via `dotnetdll`, builds a minimal .NET assembly in memory, and executes it through the VM's `Executor`.

**Key types** (defined in `crates/dotnet-vm/src/fuzzing.rs`):

- **`FuzzProgram`**: Top-level input — a list of `FuzzInstruction`s plus `num_locals` (0–10) and `num_args` (1–10).
- **`FuzzInstruction`**: An enum of CIL instruction variants covering:
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

Fuzzes `ManagedPtr::offset()` over unmanaged, Stack, and Static origin classes and offset deltas.
The harness derives every pointer from live target-local storage, then asserts that origin, offset,
and the address derived from that allocation are preserved; it filters deltas outside the fixture.

### `fuzz_managed_ptr_roundtrip` (dotnet-value)

**Location:** `crates/dotnet-value/fuzz/fuzz_targets/fuzz_managed_ptr_roundtrip.rs`

Fuzzes the `ManagedPtr` write/read serialization roundtrip using live fixture-backed origins.
It asserts origin, offset, and resolved-address preservation, and verifies that checksum-valid unknown
serialized subtags return `PointerDeserializationError::UnknownSubtag` rather than crashing.
The decoded representation is `ManagedPtrInfo`; fuzz input never supplies one with a fabricated GC
handle or executable pointer.

### `fuzz_raw_memory_access` (dotnet-value)

**Location:** `crates/dotnet-value/fuzz/fuzz_targets/fuzz_raw_memory_access.rs`

Fuzzes `AtomicAccess::store_atomic` / `load_atomic` with arbitrary offsets, sizes (1/2/4/8 bytes), values, and memory orderings. Validates that a store followed by a load returns the correctly masked value.

## Blocking Corpus Regression

The replay gate covers the targets in `fuzz_matrix` from `scripts/ci_select.py`; each has a
committed, nonempty corpus. `fuzz_managed_ptr_rebase` has no committed corpus and is not gated.
To replay locally, from the indicated working directories:

```bash
cd crates/dotnet-value
ASAN_OPTIONS=detect_leaks=0 cargo +"$NIGHTLY" fuzz run fuzz_managed_ptr_roundtrip fuzz/corpus/fuzz_managed_ptr_roundtrip -- -runs=0
ASAN_OPTIONS=detect_leaks=0 cargo +"$NIGHTLY" fuzz run fuzz_managed_ptr_offset fuzz/corpus/fuzz_managed_ptr_offset -- -runs=0
ASAN_OPTIONS=detect_leaks=0 cargo +"$NIGHTLY" fuzz run fuzz_raw_memory_access fuzz/corpus/fuzz_raw_memory_access -- -runs=0
cd ../dotnet-vm/fuzz
ASAN_OPTIONS=detect_leaks=0 cargo +"$NIGHTLY" fuzz run fuzz_executor corpus/fuzz_executor -- -runs=0
```

CI wraps each replay in `scripts/replay_fuzz_corpus.py`. It requires the Git-index inventory of
the corpus to be nonempty and unchanged, with regular nonempty tracked files, and requires the
loaded seed count to equal that inventory. The child must exit successfully, and libFuzzer's
INITED, DONE and final execution counts must agree and be at least N+1. libFuzzer may rerun inputs,
so the count need not equal N+1, and `corp:` representatives are not execution counts. Leak
detection is disabled because the runners are sandboxed.

## Seed Corpus & Dictionary

### Seed Corpus

Seed inputs for `fuzz_executor` live in `crates/dotnet-vm/fuzz/corpus/fuzz_executor/`; the value
targets' corpora are under `crates/dotnet-value/fuzz/corpus/`.

Keep each gated corpus nonempty: the CI replay explicitly
checks this before invoking libFuzzer. Crash artifacts remain local under `artifacts/`; minimize a
relevant one and add the minimized input to its target's corpus before relying on it as a regression.

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
cargo +"$NIGHTLY" fuzz run fuzz_executor

# With corpus, dictionary, and tuned options
cargo +"$NIGHTLY" fuzz run fuzz_executor \
    fuzz/corpus/fuzz_executor \
    -- \
    -dict=fuzz/cil.dict \
    -timeout=2 \
    -rss_limit_mb=2048 \
    -max_len=10000 \
    -use_value_profile=1 \
    -print_coverage=1

# Fixed-duration run (60 seconds)
cargo +"$NIGHTLY" fuzz run fuzz_executor \
    fuzz/corpus/fuzz_executor \
    -- -max_total_time=60

# With AddressSanitizer (recommended for finding memory bugs)
RUSTFLAGS="-Zsanitizer=address" \
    cargo +"$NIGHTLY" fuzz run fuzz_executor \
    fuzz/corpus/fuzz_executor
```

### dotnet-value fuzzing

```bash
cd crates/dotnet-value

cargo +"$NIGHTLY" fuzz run fuzz_managed_ptr_offset
cargo +"$NIGHTLY" fuzz run fuzz_managed_ptr_roundtrip
cargo +"$NIGHTLY" fuzz run fuzz_raw_memory_access
```

### Listing available targets

```bash
cargo +"$NIGHTLY" fuzz list
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
cargo +"$NIGHTLY" fuzz cmin fuzz_executor fuzz/corpus/fuzz_executor
```

### Coverage

Inspect which code paths are being exercised:

```bash
cargo +"$NIGHTLY" fuzz coverage fuzz_executor fuzz/corpus/fuzz_executor
```

## Crash Artifacts

When the fuzzer discovers a crash (panic, assertion failure, sanitizer violation), the triggering input is saved to:

- `crates/dotnet-vm/fuzz/artifacts/fuzz_executor/crash-<hash>`
- `crates/dotnet-value/fuzz/artifacts/<target>/crash-<hash>`

To reproduce a crash:

```bash
cargo +"$NIGHTLY" fuzz run fuzz_executor fuzz/artifacts/fuzz_executor/crash-<hash>
```

Crash artifacts and generated inputs are ignored by default; the gated corpus directories are
explicitly tracked exceptions. The advisory workflow
also runs `corpus-tools` before its executor campaign; that does not replace the checked-in replay corpus.
Minimize and deliberately add a relevant input to a tracked gated corpus, or add a harness
unit test in `crates/dotnet-vm/src/fuzzing.rs` for a persistent regression.

## Harness unit tests

The module `crates/dotnet-vm/src/fuzzing.rs` (compiled only with the `fuzzing` feature) contains
unit tests that run `execute_cil_program` on hand-written `FuzzProgram` inputs: arithmetic,
division by zero, stack underflow, locals, null throw, out-of-bounds branches, floats, native-int
locals, bitwise operations, `localloc` with indirect access, and arrays. They run natively in the
`fuzzing` leg of the CI test matrix. Under Miri they switch to a mock `AssemblyLoader` that needs
no .NET runtime, but no CI job runs them under Miri.

```bash
cargo test -p dotnet-vm --features fuzzing fuzzing
cargo +"$NIGHTLY" miri test -p dotnet-vm --features fuzzing fuzzing
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
