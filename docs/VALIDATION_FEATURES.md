# Validation Features

`dotnet-rs` provides several compile-time features to enable runtime validation of various components. These are essential for debugging and ensuring the correctness of the runtime, especially in multithreaded scenarios.

## Feature Matrix

The following table describes the available validation features and which crates expose them.

| Feature | Description | Primary Crates |
|---|---|---|
| `multithreading` | Enables thread-safe atomics, mutexes, and multi-threaded GC. (Default) | `dotnet-cli`, `dotnet-vm`, `dotnet-runtime-resolver`, `dotnet-runtime-memory`, `dotnet-pinvoke`, `dotnet-tracer`, `dotnet-utils`, `dotnet-value` |
| `memory-validation` | Enables extra checks for GC-managed pointers and arena boundaries. | `dotnet-cli`, `dotnet-vm`, `dotnet-runtime-memory`, `dotnet-utils`, `dotnet-value` |
| `metadata-validation` | Validates ECMA-335 §II.22 metadata table/name constraints (non-empty module/assembly/typedef/field/method names, direct self-inheritance) during assembly load. | `dotnet-cli`, `dotnet-vm`, `dotnet-assemblies` |
| `generic-constraint-validation` | Enforces generic type constraints during type resolution and JIT. | `dotnet-cli`, `dotnet-vm`, `dotnet-runtime-resolver`, `dotnet-intrinsics-reflection`, `dotnet-types` |
| `validation-all` | Meta-feature enabling all the above validation features. | `dotnet-cli`, `dotnet-vm` |
| `fuzzing` | Enables `Arbitrary` implementations and fuzzing-specific instrumentation. | `dotnet-cli`, `dotnet-vm`, `dotnet-pinvoke`, `dotnet-utils`, `dotnet-value`, `dotnet-types` |

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

The following tests are conditionally ignored under `memory-validation` because their deliberate
managed-pointer offsets trigger its debug bounds trap:

- VM control `p13_ldelema_managed_add_far_out_of_bounds_then_conv_u`;
- VM regression `conv_u8_after_far_oob_managed_add_does_not_panic`;
- value regression `ptr_arith_managed_ptr_arithmetic_boundaries_report_explicit_errors`.

The pinned Miri lanes run without that feature, so these tests execute there.

## Debugging Environment Variables

In addition to feature flags, some components use environment variables for extra tracing:

- `DOTNET_TRACE_GC_PTR_READ=1`: Enables tracing of every GC pointer read (very verbose).
- `DOTNET_RS_TRACE=info`: Configures the global tracing level (uses `env_filter` syntax).
- `DOTNET_RS_TRACE_FORMAT=json`: Changes tracing output format to JSON.

Example:
```bash
DOTNET_RS_TRACE=dotnet_vm=debug cargo test --features validation-all
```
