# Validation Features

`dotnet-rs` provides several compile-time features to enable runtime validation of various components. These are essential for debugging and ensuring the correctness of the runtime, especially in multithreaded scenarios.

## Feature Matrix

The following table describes the available validation features and which crates expose them.

| Feature | Description | Primary Crates |
|---|---|---|
| `multithreading` | Enables thread-safe atomics, mutexes, and multi-threaded GC. (Default) | `dotnet-cli`, `dotnet-vm`, `dotnet-utils`, `dotnet-value`, `dotnet-vm-ops` |
| `memory-validation` | Enables extra checks for GC-managed pointers and arena boundaries. | `dotnet-cli`, `dotnet-vm`, `dotnet-value`, `dotnet-vm-ops` |
| `metadata-validation` | Validates .NET metadata (tables, blobs, signatures) during assembly load. | `dotnet-cli`, `dotnet-vm`, `dotnet-assemblies` |
| `generic-constraint-validation` | Enforces generic type constraints during type resolution and JIT. | `dotnet-cli`, `dotnet-vm`, `dotnet-types` |
| `validation-all` | Meta-feature enabling all the above validation features. | `dotnet-cli`, `dotnet-vm` |
| `fuzzing` | Enables `Arbitrary` implementations and fuzzing-specific instrumentation. | `dotnet-cli`, `dotnet-vm`, `dotnet-utils`, `dotnet-value`, `dotnet-types`, `dotnet-vm-ops` |

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

The CI pipeline (`.github/workflows/ci.yml`) runs several "legs" to ensure coverage across different feature combinations.

| CI Job Name | Feature Combination | Purpose |
|---|---|---|
| `No features` | `(none)` | Verifies basic single-threaded operation without any instrumentation. |
| `Multithreading` | `multithreading` | Core concurrency testing (Default). |
| `Generic constraint validation` | `generic-constraint-validation` | Verifies type system correctness and constraint enforcement. |
| `Memory validation` | `memory-validation` | Targeted testing of GC safety and memory layout. |
| `Metadata validation` | `metadata-validation` | Stress tests the assembly loader and metadata validator. |
| `Multithreading + Memory validation` | `multithreading,memory-validation` | High-stress concurrency and memory safety checks. |
| `Full validation` | `multithreading,validation-all` | Comprehensive validation of all components. |

## Debugging Environment Variables

In addition to feature flags, some components use environment variables for extra tracing:

- `DOTNET_TRACE_GC_PTR_READ=1`: Enables tracing of every GC pointer read (very verbose).
- `DOTNET_TRACE_CULTUREDATA_WRITES=1`: Enables tracing of culture-specific data initialization.
- `DOTNET_RS_TRACE=info`: Configures the global tracing level (uses `env_filter` syntax).
- `DOTNET_RS_TRACE_FORMAT=json`: Changes tracing output format to JSON.

Example:
```bash
DOTNET_RS_TRACE=dotnet_vm=debug cargo test --features validation-all
```
