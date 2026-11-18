# Validation Features

`dotnet-rs` provides several compile-time features to enable runtime validation of various components. These are essential for debugging and ensuring the correctness of the runtime, especially in multithreaded scenarios.

## Feature Matrix

The entry points are `dotnet-cli` and `dotnet-vm`; each forwards a feature to every crate that
defines it (see each crate's `Cargo.toml`). `multithreading` is a default feature of `dotnet-cli`
and `dotnet-vm` only; workspace dependencies are declared with `default-features = false`.

| Feature | Description |
|---|---|
| `multithreading` | Enables thread-safe atomics, mutexes, and multi-threaded GC. (Default) |
| `memory-validation` | Enables extra checks for GC-managed pointers and arena boundaries. |
| `metadata-validation` | Validates ECMA-335 §II.22 metadata table/name constraints (non-empty module/assembly/typedef/field/method names, direct self-inheritance) during assembly load. |
| `generic-constraint-validation` | Enforces generic type constraints during type resolution and JIT. |
| `validation-all` | Meta-feature enabling all the above validation features. |
| `fuzzing` | Enables `Arbitrary` implementations and fuzzing-specific instrumentation. |

Other features that are not validation modes:

| Feature | Description |
|---|---|
| `simd` | SIMD byte-operation paths in `dotnet-simd` and the string intrinsics (part of the shared feature matrix). |
| `heap-diagnostics` | `HeapManager` keeps an address-ordered map of every object (`_all_objs`) instead of its compact registry, for heap inspection. |
| `deadlock-diagnostics` | Enables `parking_lot`'s deadlock detection; implies `multithreading`. |
| `bench-instrumentation` | Benchmark counters (allocation pressure, cache key clones, GC pauses) used by `dotnet-benchmarks`. |

`./check.sh --full` runs the tests with each of the last three ([CI.md](CI.md#checksh)).

## Atomic Alignment

Alignment validation for Interlocked read-modify-write operations is always enabled:
`RawMemoryAccess::compare_exchange_atomic`, `exchange_atomic`, and
`exchange_add_atomic` reject misaligned addresses before invoking Rust atomic APIs. This is a
safety guard rather than a `memory-validation` diagnostic, and an Interlocked caller receives a
managed `DataMisalignedException`. Misaligned volatile loads and stores remain valid and use a
lock-guarded memcpy fallback instead of throwing.

`dotnet_utils::validate_alignment` remains available as an opt-in validation-mode hook and is a
no-op outside `memory-validation`; it is not a shipping guard. Low-level `StackValue` atomic
operations enforce their unsafe alignment contracts in debug builds, while managed instruction
paths perform unconditional checks in `RawMemoryAccess`. `FieldStorage` also checks its layout
invariant in debug builds and retains the atomic layer's lock-guarded fallback as defense in depth.

## Recommended Local Commands

Depending on what you are debugging, use the following feature flags with `cargo test`.

### Full Validation Suite
Exercise enabled validation paths with this command. It does not cover every feature-disabled
branch or establish race freedom; select isolated configurations according to the change.
```bash
cargo test -p dotnet-cli --features validation-all
```

### Memory and Race Conditions
If you suspect memory corruption or race conditions in the GC, use this combination.
```bash
cargo test -p dotnet-cli --features multithreading,memory-validation
```

### Metadata Issues
If the runtime is failing to load a specific assembly or crashing during metadata parsing.
```bash
cargo test -p dotnet-cli --features metadata-validation
```

### Generic Type System
If you are working on the type system or generics.
```bash
cargo test -p dotnet-cli --features generic-constraint-validation
```

Compact cross-arena `ObjectRef` owner/address limits, unsigned ID decoding and checked VM ID
exhaustion are enforced in release, independent of `memory-validation`. Exact identity checks
are not a substitute for rooting or the unsafe live-original-allocation contract; magic/arena
validation diagnostics cannot safely validate a stale pointer. See [GC and Memory Safety](GC_AND_MEMORY_SAFETY.md#cross-arena-reference-tracking).

## Feature configurations in CI

The `test` and `clippy` jobs in `.github/workflows/ci.yml` run every combination in
`SHARED_FEATURE_MATRIX` (`xtask/src/main.rs`); print it with
`cargo run -p xtask -- matrix test-features`. Each leg builds with `--no-default-features`
plus the listed features. Miri lanes are separate explicit jobs; [CI.md](CI.md) maps each job to
its source of truth, and `./check.sh --fast` runs a smaller local subset.

Combined validation does not replace an isolated feature leg: select the configuration whose
`cfg` branches you changed.

## Debugging Environment Variables

In addition to feature flags, some components use environment variables for extra tracing:

- `DOTNET_TRACE_GC_PTR_READ=1`: Enables tracing of every GC pointer read (very verbose).
- `DOTNET_RS_TRACE`: Selects `stdout` (also `1` or `true`), `stderr`, or a file path for trace output.
- `RUST_LOG`: Configures trace levels with `env_filter` syntax; `DOTNET_RS_TRACE_LEVEL`, when set,
  instead selects a predefined level for VM trace targets.
- `DOTNET_RS_TRACE_METHODS`: Optional comma-separated method-name substrings selecting method
  entry and exit events only; other event categories are unaffected. An empty list selects all.
- `DOTNET_RS_TRACE_FORMAT=json`: Changes tracing output format to JSON.

Example:
```bash
DOTNET_RS_TRACE=stderr RUST_LOG=dotnet_vm=debug cargo test -p dotnet-cli --features validation-all
```
