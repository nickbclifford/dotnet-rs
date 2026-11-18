# Executor teardown ownership and panic safety

**Status:** Complete
**Outcome:** Dropping or extracting an `Executor` never destroys another Executor's arena or
frees storage still protected by an in-flight lease, including during unwind.

The contract lives in
[GC and Memory Safety — Per-thread arenas](../GC_AND_MEMORY_SAFETY.md#per-thread-arenas)
(invariant `F1.ExecutorArenaRetained`). The regression is
`executor::executor_teardown_tests::extracted_executor_tls_ownership`.

The real-VM collection fixture's foreign-branded `Gc`, which this plan left open, was replaced by
production-codec transport in [plan 06](06-cross-arena-collection-fixture.md).
