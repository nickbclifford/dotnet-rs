# Pointer / provenance review evidence

Evidence record, not instructions; current work is in [docs/plans/](../plans/README.md).

This folder records the outcomes of the pointer/provenance review and its repairs (heap access,
pointer arithmetic, frame-owned `localloc` blocks, frame homes, and owner/guard retention).
[INVENTORY.md](INVENTORY.md) defines the probe IDs (p00…p44 and the named probes), the finding
IDs (R01–R13) and the evidence vocabulary, and maps each finding to its repair or to an open
entry in the plans README. The current runtime contracts are in
[GC and Memory Safety](../GC_AND_MEMORY_SAFETY.md) and
[Architecture](../ARCHITECTURE.md#evaluation-stack-locals-and-managed-pointers).

**Limits of the evidence.** Pointer-lane Miri results come from a filtered, permissive Tree
Borrows lane with leaks ignored, on the pinned nightly. They are not strict-provenance evidence,
a full VM Miri suite, leak checking or a soundness proof. Several original review archives are not
in this repository; tests that reproduce their reported intents are independent recreations.

**Open items.** `p12_ldsflda_convu_stind_static` (a static field address converted to a native
integer and stored through) is classified suspected-ub and stays a diagnostic probe;
`p_wide_store_into_int_local` is unverified. Whether the byte-`Fill` observation-to-access gap was
reachable end to end was never confirmed. The open findings are tracked under
[Not yet planned](../plans/README.md#not-yet-planned).

**Where things live.** `bash scripts/pointer_provenance.sh list` prints the catalog (blocking
controls, diagnostic-only probes and shards); [`pointer_provenance.sh`](../../scripts/pointer_provenance.sh)
owns discovery, guarded control shards and bounded diagnostics, and
[`pointer_provenance_tests.rs`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) is the
VM test source. The CI lane layout is in [CI](../CI.md#blocking-miri-pointer-controls-lane).
Detailed historical measurements are in git.
