# Executor teardown ownership and panic safety

**Status:** Complete
**Outcome:** Dropping or extracting an `Executor` never destroys another Executor's arena or
frees storage still protected by an in-flight lease, including during unwind.

Implemented in `ed9a636c`. The contract now lives in
[GC and Memory Safety — Per-thread arenas](../GC_AND_MEMORY_SAFETY.md#per-thread-arenas)
(invariant `F1.ExecutorArenaRetained`). The regression is
`executor::executor_teardown_tests::extracted_executor_tls_ownership`.

Not covered by this plan: the real-VM collection fixture's foreign-branded `Gc` (see
[open items in the pointer-provenance inventory](../pointer-provenance/INVENTORY.md)).
