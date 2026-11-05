# Stable frame homes: slice 4 acceptance (2026-11-05)

Slice 4 of plan 01 is complete. The Stack origin, serialized/cached rebasing,
evaluation-stack fixup, scratch caches, unused slot-address APIs, fixup metrics and
rebasing fuzz target are removed. Managed pointers remain 24 bytes; retired Stack
encodings fail decoding, and surviving encodings keep their tags.

TypedReference address-taking raises InvalidProgramException under the selected strict
ECMA policy. Ordinary typedrefs retain pointer provenance, descriptor ownership and GC
tracing. p42 checks initialized/uninitialized local and argument addresses, special and
named signatures, ordinary copying/type extraction/dereferencing, collection with an
array rooted only through a typedref, and InvalidCastException for a mismatched type.

## Validation

- `./check.sh --fast`: passed all configurations, including clippy and fixture tests.
  No features: 612 passed, 6 skipped. Multithreading: 732 passed, 5 skipped.
  Multithreading plus validation-all: 747 passed, 7 skipped.
- `scripts/pointer_provenance.sh batch`: batches 1–4 passed in both no-features
  and multithreading configurations, covering all 42 controls. This includes p01,
  p03, p09, p15, assignment/lifecycle controls and constructor receiver controls.
- The final expanded p42 also passed singleton shard 42 in both configurations.
- Pinned `nightly-2026-05-27` Miri library suites: dotnet-value and dotnet-vm-data
  passed, using Tree Borrows, disabled isolation and ignored leaks as prescribed by CI.
- Native dotnet-value, dotnet-vm-data and dotnet-vm library tests passed. The metrics
  library tests passed with bench-instrumentation; the managed-pointer benchmarks compile.
- Clippy passed for VM, value, VM-data and metrics with all targets and
  multithreading, validation-all and bench-instrumentation, treating warnings as errors.
- Committed managed-pointer roundtrip and offset corpora replayed successfully with
  the pinned nightly and `-runs=0`; all remaining value fuzz targets compile.
- Pointer-runner tests: 21 passed. Documentation drift: 49 passed, 0 failed.
- Source search confirms no Stack origin, stack-base resolver hook, fixup or rebasing
  implementation remains. `git diff --check` passes.

The ECMA-335 MCP search corroborated the typed-reference signature restrictions in
I.8.6.1.3. The rejection policy is a deliberate compatibility boundary; this work does
not claim support for CoreCLR's unsafe typedref address-taking behavior.

Plan 01 remains in progress for slice 5 performance measurement. General bulk-operation
barriers/yield state and static-address provenance remain separate work.
