# Tools and benchmark cleanup

Dispositions for findings TOOLS-1 through TOOLS-7 of the [27a audit](../27a-test-audit.md) (the
finding texts are in git), implemented under [27b](../27b-test-consolidation.md).

| Finding | Resolution and retained contract |
|---|---|
| TOOLS-1 | Pointer control totals and allowed shard/batch identities derive from the named registry. Catalog agreement, exact singleton selection, discovery/ignore checks and Rust batch membership remain required. Runner tests exercise incomplete registration, non-singleton registration and a fully registered addition; only the complete addition passes. |
| TOOLS-2 | Advisory shards derive from the manifest instead of fixed ranges or totals. Canonical IDs, selector validity, uniqueness, configuration coverage and agreement with actual discovery remain required. Synthetic growth passes without a separate threshold edit; missing and unregistered discovered cases fail. |
| TOOLS-3 | Cross-arena workflow tests check named selectors and derive invocation totals from the required sets. Duplicate matrix rows and missing or incorrect selectors still fail. No global matrix-row census remains. |
| TOOLS-4 | Removed the repository-wide empty-Markdown-include assertion. Selection tests still verify that includes select their owning lanes, including nested dependencies and failure cases. |
| TOOLS-5 | Clippy's no-dotnet contract observes actual invocations through a PATH shim. A quiet invocation fails even when its exit status is ignored; mentioning project filenames in output succeeds, and unrelated command failure remains a failure. The build-script probe can be sourced to test this real guard without invoking its clean-build main routine. |
| TOOLS-6 | Managed-pointer serialization benchmarks validate each decoded result before timing, including Transient's explicit rejection. The frame-home resolver reports its actual backing allocation extent. Cache preflight checks the expected pointer across two distinct objects and a return to the first object. The smoke run exposed a mistaken expected Transient subtag in the new assertion; inspection of the documented wire encoding established subtag 2, and the corrected smoke passes. |
| TOOLS-7 | Overlap workloads use `Span.CopyTo`, whose contract supports overlap. Disjoint `Unsafe.CopyBlock` workloads remain, with boundary values checked. Both changed managed fixtures build and return success on the CLR. |

## Related execution wiring

The pointer Miri prefix lane derives its required pass total from the required names and checks
each name in output. It includes the live-allocation offset control; the isolated strict-provenance
lane runs that control separately. Library selector renames and removal of a redundant publication
test are reflected in the advisory manifest and cross-arena workflow without relaxing exact-run
guards.

FieldType compile-fail checks use the existing diagnostic checker with a compiling positive
control, rustdoc-equivalent snippet wrapping and exact diagnostic-code validation. CI runs the
checker and doctests in both thread configurations. This implements the tooling part of LIB-15 ([library cleanup](cleanup-libraries.md)); stable
rustdoc cannot check `compile_fail` error numbers itself.

`CONTRIBUTING.md` records the accepted behavioral/internal contract criteria, independent oracles,
specific failure evidence and deriving execution counts from named required selections.
