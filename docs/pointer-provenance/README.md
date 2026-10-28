# Pointer / provenance review evidence and current status

The pointer/provenance plan remains **In progress — partial M0**. The M1a heap-access containment implementation and its scoped evidence are now present, but this does not complete M0, the broader M1 exits, or M3. See the [unified plan](../plans/01-pointer-provenance.md) and the [durable evidence inventory](INVENTORY.md) for the baseline table, probe catalog, R-row task status, and detailed results.

## M1a outcomes

The pre-repair Miri baseline covered p10, p13, p14, p16 and p20 in no-features and multithreading. All ten separate runs were classified **miri-ub** (exit 1): Miri reported a dangling, out-of-allocation `&[u8]` at `crates/dotnet-value/src/pointer/mod.rs:505:37`; the caller was `conversions.rs:101:21` for p10/p13/p20 and `stack_value.rs:574:30` for p14/p16. No-features wall times were 268/233/236/241/245 seconds, respectively; multithreading times were 243/277/258/259/244 seconds. The source commits were `775886fc7554f9d5b58014f2b3db30c2e17d1982` (no-features) and `9524740e01953416a5d68eeee377eb18c9f51851` (multithreading; intervening changes were tracking-only). The full per-probe table is in [INVENTORY.md](INVENTORY.md#pre-repair-baseline-steps-112).

After the heap repairs, all five probes passed their individual diagnostics in both configurations under pinned permissive Tree Borrows. They were promoted as blocking controls without changing their VM probe bodies, IL or assertions (p10=7, p13=1, p14/p16=`0x01010101`, p20=7). The p13 test retains a `memory-validation`-conditional ignore because the deliberate 4096-byte managed offset is rejected by that debug trap; the Miri control lane does not enable memory-validation. The separate far-OOB `conv.u8` regression is conditionally ignored under the same feature for the same trap; it passed pinned Miri in both configurations without that feature.

Three library regressions in `crates/dotnet-value/src/pointer/tests.rs` passed individually under the same pinned lane in both feature configurations, each with 1 passed/0 failed/0 ignored:

- `pointer::tests::m1a_one_byte_heap_read`: reads exactly one byte (`0xA5`) from a one-byte heap payload.
- `pointer::tests::m1a_safe_oob_address_observation`: checks the wrapping address at logical offset 4096 through both `PointerLike::pointer()` and `StackValue::ManagedPtr::as_ptr()` without dereferencing.
- `pointer::tests::m1a_heap_pointer_survives_typed_write`: for array, object and boxed payloads, checks that a retained heap base survives a guarded typed write of 5, a raw write of 7, and a guarded readback of 7.

The original pass-1 library sources and pass archives remain unavailable. The first two tests independently recreate the reported intents and make **no identity claim** about the missing originals; the typed-write test is a new regression.

The blocking pointer runner now has **11 exact controls**, in six shards, across no-features and multithreading. Final `verify-controls` passed in both configurations; every shard passed (2 controls in shards 1–5, 1 in shard 6). The largest recorded local shard wall was 1328.707 seconds, below the 2700-second limit, but the measurements were warm and contended and do not guarantee hosted CI timing. `miri-value` CI includes the three M1a library selectors in both its default multithreading run and a separate no-features step. The [inventory](INVENTORY.md#m1a-beforeafter-outcomes-and-repaired-scope) includes the exact shard walls and the evidence caveat.

## Repaired heap paths and remaining boundaries

M1a repaired heap `ManagedPtr::with_data` to validate checked ranges against actual payload extent and create only the requested slice; separated heap address observation from access using retained allocation bases and wrapping formation; routed heap `StackValue::as_ptr` and VM `resolve_address` through that observer; fixed both distinct integer conversions, `conv.u` and `conv.u8`; fixed heap-origin `AsPointer`/Unsafe and buffer address helpers; and kept retained-base derivation independent of a payload-slice reborrow so pointers survive ordinary typed writes.

The following remain out of scope: Stack/Static observation and the `with_data` stack fallback (M3); any writable contract for strings/interned Arc data; P/Invoke pinning, guards, ABI and return/copy semantics (M2); and general `cpblk`/`initblk` correctness (M1f/M5: count width, full checked ranges, barriers/rooting and yield state). The short valid p14/p16 block probes only validate the M1a address path. The legacy `StackValue` managed-pointer/native-integer comparisons and managed-pointer subtraction still form Heap addresses with in-bounds `.add`; a far-OOB operand can violate that premise without a dereference. These operations remain an M1b follow-up, not covered by the repaired observation tests. Also open are pointer-codec/struct-layout redesign, M4 shared-memory/atomic protocol, M1b `conv.i`/pointer semantics, M1d `pop_ptr`/Transient owner-guard escape, M1e/M1g GC transport and identity, and strict-provenance lint migration/evidence recovery. R01's M1a heap reproduction is closed; broader R01/M3 and R06/R07/R10 follow-ups remain as detailed in the [R-row map](INVENTORY.md#finding-to-work-map).

## Historical evidence and limits

The original pass-1/2/3 archives and raw pass logs, standalone lab/FFI sources, and the 3c2 patch are **still absent** from this checkout; an external backup has not been ruled out. In particular, p20's historical patched source is not evidence of an original pass. The unpatched-tree p20 baseline above was independently observed in steps 1.1/1.2.

All pointer-lane Miri results here are from a **filtered permissive Tree Borrows lane**. This is not strict-provenance evidence, not a full VM Miri suite, and not a proof of soundness. Native benchmark numbers are performance measurements, not Miri evidence. The step 5.1 benchmark review recorded native Criterion comparisons and repeated no-features regressions; those regressions remain an open performance follow-up, not a Miri result.

## Current executable records and safe operation

- [`INVENTORY.md`](INVENTORY.md) records the outcomes and limits, R01–R13 task updates, all 23 exact probe names, source variants, current line anchors, control results and still-open work.
- [`pointer_provenance_tests.rs`](../../crates/dotnet-vm/src/pointer_provenance_tests.rs) is the owning VM test source; [`pointer_provenance.sh`](../../scripts/pointer_provenance.sh) owns catalog discovery, guarded control shards and bounded diagnostics.
- Only p17/p18/p19 are permitted for native diagnostics. Never run known-UB probes natively or ignored tests outside the runner's diagnostic entrypoint.
- The source test anchors in the inventory are manually refreshed; they are not machine-checked.
