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

## Miri CI Lane Mapping

The feature matrix above describes the dynamic stable test/clippy jobs; Miri jobs are separate,
explicit workflow lanes and are not represented by `SHARED_FEATURE_MATRIX`.

| CI job | Blocking? | Configuration / selection | Meaning and limits |
|---|---|---|---|
| `miri-value` in `ci.yml` | Yes | Pinned `nightly-2026-05-27`; full `dotnet-value` test suite with default features (multithreading), plus no-default-features selectors `pointer::tests::m1a_` (3 tests) and `pointer::tests::m1b_` (8 tests) | Blocking full value-crate Miri coverage in the default configuration. The no-features selectors cover exactly the three M1a and eight M1b library regressions, not the full crate; neither step is a VM gate. |
| `miri-pointer-controls` in `ci.yml` | Yes | Pinned `nightly-2026-05-27`; `MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation -Zmiri-ignore-leaks"`; `--no-default-features` with and without `--features multithreading`; twelve guarded shards per configuration | Twenty-four enabled exact VM controls: shard 1 p00/p02; 2 p03/p05; 3 p07/p11; 4 p10/p20; 5 p13/p14; 6 p16/p_null; 7 p06/p08; 8 p17/p18; 9 p21/p22; 10 p23/p24; 11 p25/p26; 12 p27/p28. Seven diagnostic-only probes are not executed by CI. This is a filtered permissive Tree Borrows lane, not a full VM Miri suite, strict-provenance validation, or proof of runtime soundness. |
| `miri` matrix in `miri.yml` | No (`continue-on-error`) | Pinned `nightly-2026-05-27`; VM entries use `jmp_tests tail_calls fault_tests` substring filters in no-features and multithreading configurations | Existing advisory VM unsafe-gate runs remain unchanged; their bounded filters do not establish whole-VM coverage. Other crate entries are also advisory. |
| Strict-provenance Miri | No job | None | No current CI job uses `-Zmiri-strict-provenance`; do not infer strict coverage from the two blocking Miri jobs. |

The following tests are conditionally ignored under `memory-validation` because their deliberate
managed-pointer offsets trigger its debug bounds trap:

- VM control `p13_ldelema_managed_add_far_out_of_bounds_then_conv_u`;
- VM regression `conv_u8_after_far_oob_managed_add_does_not_panic`;
- value regression `m1b_managed_ptr_arithmetic_boundaries_report_explicit_errors`.

They run in the pinned Miri configurations without that feature.

The pointer-control job uses a provisional 45-minute timeout per matrix leg. Hosted cold-build/cache
timings have not been measured. Ignored pointer probes may only be executed through the explicit
diagnostic entrypoint in `scripts/pointer_provenance.sh`; native diagnostics are limited to catalog
entry p19.

## Debugging Environment Variables

In addition to feature flags, some components use environment variables for extra tracing:

- `DOTNET_TRACE_GC_PTR_READ=1`: Enables tracing of every GC pointer read (very verbose).
- `DOTNET_RS_TRACE=info`: Configures the global tracing level (uses `env_filter` syntax).
- `DOTNET_RS_TRACE_FORMAT=json`: Changes tracing output format to JSON.

Example:
```bash
DOTNET_RS_TRACE=dotnet_vm=debug cargo test --features validation-all
```
