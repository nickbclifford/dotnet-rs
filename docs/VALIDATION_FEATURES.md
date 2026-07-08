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

## Recommended Local Commands

Depending on what you are debugging, use the following feature flags with `cargo test`.

### Full Validation Suite
Run all tests with every validation feature enabled to catch any hidden issues.
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

## CI Leg Mapping

The CI pipeline (`.github/workflows/ci.yml`) does not hardcode named legs. Instead, a single
`Test Suite` job (and a parallel `Clippy Check` job) runs over a dynamic matrix resolved at
runtime from `cargo run -p xtask -- matrix test-features --format json`. The matrix is the
`SHARED_FEATURE_MATRIX` constant in `xtask/src/main.rs` — that constant is the source of truth,
so consult it directly to avoid drift. As of this writing it contains the following legs (each run
with `cargo test --no-default-features [--features <combo>]`):

- `(none)`
- `multithreading`
- `simd`
- `multithreading,simd`
- `generic-constraint-validation`
- `memory-validation`
- `metadata-validation`
- `multithreading,memory-validation`
- `multithreading,validation-all`
- `fuzzing`

## Debugging Environment Variables

In addition to feature flags, some components use environment variables for extra tracing:

- `DOTNET_TRACE_GC_PTR_READ=1`: Enables tracing of every GC pointer read (very verbose).
- `DOTNET_RS_TRACE=info`: Configures the global tracing level (uses `env_filter` syntax).
- `DOTNET_RS_TRACE_FORMAT=json`: Changes tracing output format to JSON.

Example:
```bash
DOTNET_RS_TRACE=dotnet_vm=debug cargo test --features validation-all
```
